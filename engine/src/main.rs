//! axyr-engine — the local agent.
//!
//! It owns the debug probe (the single, serial SWD resource) on one background
//! thread that drains RTT telemetry and runs board actions (see [`agent`]). The
//! main thread is the MCP front-end (stdio, one JSON-RPC message per line): it
//! serves read tools directly and routes action tools to the owner thread over a
//! channel, so an agent's actions never disturb the real-time telemetry.
//!
//! A second front-end — an HTTP API (see [`serve_http`]) for the web dashboard —
//! runs on its own thread and routes through the SAME dispatch, so the dashboard
//! and an MCP agent always see the same view of the system.

use std::env;
use std::fs;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use serde_json::json;

use axyr_engine::agent::{self, Command, Config};
use axyr_engine::api::{self, Shared};
use axyr_engine::coredump::CoredumpTools;
use axyr_engine::probe::{Detection, Probe};
use axyr_engine::chip as chipid;
use axyr_engine::link::LocalLink;
use axyr_engine::relay;
use axyr_engine::threads::ThreadTable;
use axyr_engine::system_map;
use axyr_engine::wire;

fn main() {
    let args: Vec<String> = env::args().collect();

    match args.get(1).map(String::as_str) {
        Some("--version" | "-V") => {
            println!("axyr {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        Some("--help" | "-h") => {
            println!("axyr {} — the local agent: plugs your board into the dashboard and MCP", env!("CARGO_PKG_VERSION"));
            println!();
            println!("usage: axyr [build-dir|firmware.elf] [crash-file]");
            println!();
            println!("  run it from your project: it finds build/zephyr/zephyr.elf, attaches to");
            println!("  the board over the debug probe (chip auto-detected), serves the dashboard");
            println!("  API on 127.0.0.1:7878 and MCP on stdio.");
            println!();
            println!("environment: AXYR_CHIP, AXYR_DTS, AXYR_SVD, AXYR_HTTP, AXYR_WATCH (see README)");
            return;
        }
        _ => {}
    }

    // `--connect <ws-url>` (or AXYR_ENGINE) switches to dumb-relay mode: the
    // probe is served raw to a remote engine, no analysis happens here.
    let mut engine_url = env::var("AXYR_ENGINE").ok();
    let mut positional: Vec<String> = Vec::new();
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        if a == "--connect" {
            engine_url = it.next().cloned();
            if engine_url.is_none() {
                eprintln!("axyr: --connect needs a URL, e.g. ws://engine-host:7879");
                std::process::exit(2);
            }
        } else {
            positional.push(a.clone());
        }
    }

    // Zero-config: auto-detect the firmware build. The first positional may be
    // an explicit ELF, a project/build directory, or absent (search the cwd).
    // This is what makes `axyr` "plug in and go" — no env vars to set.
    let (elf, dts_found) = match resolve_build(positional.first().map(String::as_str)) {
        Some(b) => b,
        None => {
            eprintln!("axyr: no firmware build found.");
            eprintln!("  run `axyr` from your project (it looks for build/zephyr/zephyr.elf),");
            eprintln!("  or pass a path:  axyr <build-dir|firmware.elf> [crash-file]");
            std::process::exit(1);
        }
    };
    let crash_file = positional.get(1).cloned().unwrap_or_else(|| {
        env::temp_dir().join("axyr-last-crash.txt").display().to_string()
    });

    // Devicetree: explicit env wins, else the one found beside the ELF.
    let dts = env::var("AXYR_DTS").ok().or(dts_found);
    // Crash backtraces use native DWARF unwinding (no toolchain needed); GDB is
    // an optional fallback when these env vars point at Zephyr's coredump tools.
    let coredump = CoredumpTools::from_env(&elf);

    // Identify the probe before attaching (for the dashboard's Connect screen).
    let probe_info = Probe::list_first();

    // Which chip? Explicit AXYR_CHIP wins; otherwise auto-detect from the
    // firmware build (CONFIG_SOC) and the silicon itself — no hardcoded target.
    let attached = match env::var("AXYR_CHIP") {
        Ok(name) => Probe::attach(&name).map(|p| {
            let chip = p.chip().to_string();
            (p, Detection { chip, method: "AXYR_CHIP override".to_string(), generic: false })
        }),
        Err(_) => Probe::attach_auto(soc_from_build(&elf).as_deref()),
    };
    let (mut probe, detection) = attached.unwrap_or_else(|e| {
        eprintln!("axyr: no debug probe / attach failed ({e}).");
        eprintln!("  plug your board's USB (its on-board ST-LINK), then run `axyr` again.");
        eprintln!("  if the chip is exotic, name it explicitly:  AXYR_CHIP=<target> axyr");
        std::process::exit(1);
    });
    let chip = detection.chip.clone();
    // Read the chip identity straight from the silicon (CPUID + STM32 IDCODE).
    let (cpuid, idcode) = probe.identity();

    // Cross-check: does the silicon agree with the chip we attached as? Saves
    // the user from flashing firmware built for another board.
    let mismatch = idcode
        .and_then(|id| chipid::st_devid_matches(&chip, id))
        .is_some_and(|ok| !ok);
    if mismatch {
        eprintln!(
            "axyr: WARNING — attached as {chip}, but the silicon reports ST device id {:#05x}.",
            idcode.unwrap_or(0) & 0xfff
        );
        eprintln!("  the firmware build may target a different board than the one plugged in.");
    }

    let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();
    let svd_path = env::var("AXYR_SVD").ok();
    // Where the agent may discover flashable firmware images (defaults to the
    // ELF's directory) — so flashing is autonomous, no human supplies a path.
    let firmware_dir = env::var("AXYR_FIRMWARE_DIR")
        .ok()
        .or_else(|| std::path::Path::new(&elf).parent().map(|p| p.display().to_string()));
    let watch: Vec<String> = env::var("AXYR_WATCH")
        .map(|s| {
            s.split(',')
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect()
        })
        .unwrap_or_default();

    // Board name from the devicetree model, for the Connect screen + banner.
    let board = dts
        .as_deref()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| system_map::model(&s));
    // Dumb-relay mode: hand the probe to the remote engine and stop thinking.
    if let Some(url) = engine_url {
        let elf_bytes = fs::read(&elf).unwrap_or_else(|e| {
            eprintln!("axyr: read {elf}: {e}");
            std::process::exit(1);
        });
        let hello = wire::Hello {
            wire_version: wire::WIRE_VERSION,
            agent_version: env!("CARGO_PKG_VERSION").to_string(),
            probe: probe_info.clone(),
            chip: chip.clone(),
            detection: detection.method.clone(),
            generic: detection.generic,
            cpuid,
            devid: idcode,
            elf_b64: B64.encode(&elf_bytes),
            dts: dts.as_deref().and_then(|p| fs::read_to_string(p).ok()),
            watch: watch.clone(),
        };
        eprintln!("axyr — relay mode (dumb agent; analysis runs on the engine)");
        eprintln!("  chip  : {chip} — {}", detection.method);
        eprintln!("  build : {elf}");
        eprintln!("  engine: {url}");
        let mut link = LocalLink::new(probe);
        link.attach_telemetry_blocking();
        relay::run(link, &hello, &url); // reconnects forever; never returns
    }

    let connect = build_connect(
        &probe_info, &elf, dts.as_deref(), board.as_deref(),
        &chip, &detection, mismatch, cpuid, idcode,
    );

    let cfg = Config {
        elf: elf.clone(),
        crash_file: crash_file.clone(),
        coredump,
        svd_path,
        chip: chip.clone(),
        dts_path: dts.clone(),
        watch,
    };
    // Shared live thread state: the agent updates it, the MCP front-end reads it.
    let threads = Arc::new(Mutex::new(ThreadTable::new()));
    let threads_agent = threads.clone();
    thread::spawn(move || {
        let mut link = LocalLink::new(probe);
        link.attach_telemetry_blocking();
        agent::run(Box::new(link), cfg, cmd_rx, threads_agent)
    });

    // Friendly startup banner — what we detected, and where the dashboard is.
    eprintln!("axyr — agent ready");
    eprintln!(
        "  probe : {}",
        probe_info
            .as_ref()
            .map(|(n, s)| format!("{n}{}", s.as_ref().map(|x| format!(" · {x}")).unwrap_or_default()))
            .unwrap_or_else(|| "?".to_string())
    );
    eprintln!("  board : {}", board.as_deref().unwrap_or(&chip));
    eprintln!("  chip  : {chip} — {}", detection.method);
    if detection.generic {
        eprintln!("          observation only; set AXYR_CHIP=<target> for flash support");
    }
    eprintln!("  build : {elf}");

    // The HTTP API: the second front-end (alongside MCP stdio) the dashboard
    // consumes. Same dispatch as MCP, so both see one truth. Localhost-only by
    // default; AXYR_HTTP=off disables it, AXYR_HTTP=<addr:port> changes it.
    let http_addr = env::var("AXYR_HTTP").unwrap_or_else(|_| "127.0.0.1:7878".to_string());
    if http_addr != "off" {
        eprintln!("  dashboard API: http://{http_addr}  (open the dashboard — it will detect this board)");
        let shared = Arc::new(Shared {
            crash_file: Mutex::new(crash_file.clone()),
            dts: Mutex::new(dts.clone()),
            firmware_dir: Mutex::new(firmware_dir.clone()),
            cmd_tx: Mutex::new(cmd_tx.clone()),
            connect: Mutex::new(connect.clone()),
            threads: threads.clone(),
        });
        thread::spawn(move || api::serve_http(&http_addr, shared));
    }

    api::serve_mcp(&crash_file, dts.as_deref(), firmware_dir.as_deref(), &cmd_tx, &threads);
}

/// Resolve the firmware build to watch: an explicit ELF, a directory to search,
/// or the current directory. Returns `(elf, dts?)`.
fn resolve_build(arg: Option<&str>) -> Option<(String, Option<String>)> {
    match arg {
        Some(p) if std::path::Path::new(p).is_file() => {
            Some((p.to_string(), sibling_dts(p)))
        }
        Some(p) if std::path::Path::new(p).is_dir() => find_build_in(p),
        Some(_) => None, // a path was given but doesn't exist
        None => find_build_in("."),
    }
}

/// Search the common Zephyr build locations under `dir` for `zephyr.elf`.
fn find_build_in(dir: &str) -> Option<(String, Option<String>)> {
    let base = std::path::Path::new(dir);
    for c in ["build/zephyr/zephyr.elf", "zephyr/zephyr.elf", "zephyr.elf"] {
        let elf = base.join(c);
        if elf.is_file() {
            let elf_s = elf.display().to_string();
            return Some((elf_s.clone(), sibling_dts(&elf_s)));
        }
    }
    None
}

/// The `zephyr.dts` sitting next to the ELF, if present.
fn sibling_dts(elf: &str) -> Option<String> {
    let p = std::path::Path::new(elf).with_file_name("zephyr.dts");
    p.is_file().then(|| p.display().to_string())
}

/// The SoC the firmware was compiled for, from Zephyr's `.config` next to the
/// ELF (`CONFIG_SOC="stm32f401xe"`) — the strongest chip-detection signal we
/// have, since it names the exact die the user's build targets.
fn soc_from_build(elf: &str) -> Option<String> {
    let p = std::path::Path::new(elf).with_file_name(".config");
    let config = fs::read_to_string(p).ok()?;
    config.lines().find_map(|l| {
        l.strip_prefix("CONFIG_SOC=")
            .map(|v| v.trim().trim_matches('"').to_string())
            .filter(|v| !v.is_empty())
    })
}

/// The JSON the dashboard's Connect screen reads (GET /connect): what board /
/// probe / build was detected, so the user confirms with one click.
#[allow(clippy::too_many_arguments)]
fn build_connect(
    probe: &Option<(String, Option<String>)>,
    elf: &str,
    dts: Option<&str>,
    board: Option<&str>,
    chip: &str,
    detection: &Detection,
    mismatch: bool,
    cpuid: Option<u32>,
    idcode: Option<u32>,
) -> String {
    json!({
        "probe": probe.as_ref().map(|(n, s)| json!({ "name": n, "serial": s })),
        "board": board,
        "chip": chip,
        "detection": detection.method,
        // Generic fallback: the chip is unknown, observation works, flash won't.
        "observation_only": detection.generic,
        // The firmware build and the silicon disagree on the chip.
        "chip_mismatch": mismatch,
        "cpuid": cpuid.map(|c| format!("{c:#010x}")),
        "dev_id": idcode.map(|c| format!("{:#05x}", c & 0xfff)),
        "build": { "elf": elf, "dts": dts },
        "ready": probe.is_some(),
    })
    .to_string()
}
