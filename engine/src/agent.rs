//! The local agent: the single owner of the debug probe.
//!
//! The SWD link is one shared, inherently serial resource, so exactly one thread
//! owns the [`Probe`] and serializes everything through it. Its loop drains RTT
//! telemetry continuously and, between drains, runs any queued actions
//! (flash / reboot / read). Because RTT is buffered on the target, running an
//! action only pauses the host's draining for the action's duration — no
//! telemetry is lost and the target's real-time behaviour is never disturbed.
//!
//! On a crash, the coredump is NOT streamed over RTT (slow): the firmware's
//! IN_MEMORY backend leaves it in a RAM buffer, and the agent reads that buffer
//! in one SWD block (the core is halted after the fault) — fast and log-free.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::coredump::{CoredumpTools, resolve_backtrace_from_bin};
use crate::probe::Probe;
use crate::recent_log::RecentLog;
use crate::rtt::Telemetry;
use crate::threads::ThreadTable;
use crate::trace;
use crate::{
    Crash, diff, format_report, health, parse_crash_line, peripheral, reason_label, symbolize,
    symbols, system_map,
};

/// How many recent telemetry lines to keep as "what was happening just before".
const RECENT_LOG_LINES: usize = 20;
/// How often to record a history tick, and how many to keep (≈ last minute).
const RECORD_INTERVAL: Duration = Duration::from_secs(1);
const HISTORY_TICKS: usize = 60;
/// Zephyr's IN_MEMORY coredump backend buffer symbol.
const COREDUMP_SYMBOL: &str = "in_memory_coredump";

/// An action the agent performs on the board, via the owned probe.
pub enum Action {
    Reboot,
    Flash(PathBuf),
    ReadMemory { address: u64, count: usize },
    /// List the chip's peripherals (discovered from the SVD).
    ListPeripherals,
    /// Decode a peripheral's live register state via the SVD.
    ReadPeripheral { name: String },
    /// List the firmware's global variables (discovered from the ELF).
    ListVariables,
    /// Read a global variable live by name (resolved from the ELF).
    ReadVariable { name: String },
    /// Read the context-switch timeline from the RAM ring buffer.
    ReadTrace,
    /// Assemble the full system snapshot (the data contract) as JSON.
    GetSnapshot,
    /// Run proactive health checks and report findings.
    GetHealth,
    /// Diff a fresh snapshot against the previous one.
    DiffSnapshot,
    /// Return the recorded history (time-series of system state) for animation.
    GetHistory,
    /// Non-intrusive trigger: poll a global until it reaches `value` (or timeout).
    WatchUntil { name: String, value: u32 },
}

/// A request to the agent: an action plus a channel to send the result back.
pub struct Command {
    pub action: Action,
    pub reply: Sender<Result<String, String>>,
}

/// Static configuration the owner loop needs for the crash pipeline.
pub struct Config {
    pub elf: String,
    pub crash_file: String,
    pub coredump: Option<CoredumpTools>,
    /// Path to the chip's SVD, for decoding peripheral registers.
    pub svd_path: Option<String>,
    /// Target chip name (for device.chip).
    pub chip: String,
    /// Path to the devicetree, for the system map.
    pub dts_path: Option<String>,
    /// Global variable names to include in the snapshot's `variables`.
    pub watch: Vec<String>,
}

/// Run the probe-owner loop forever: drain RTT, capture crashes, run commands.
/// `threads` is shared with the MCP front-end so `get_threads` can read it.
pub fn run(
    mut probe: Probe,
    cfg: Config,
    commands: Receiver<Command>,
    threads: Arc<Mutex<ThreadTable>>,
) {
    // Where the IN_MEMORY coredump buffer lives in RAM (resolved from the ELF).
    let coredump_addr = symbols::resolve(&cfg.elf, COREDUMP_SYMBOL)
        .map(|s| s.address)
        .map_err(|e| eprintln!("agent: no coredump buffer symbol ({e}); call stack disabled"))
        .ok();

    // The chip's SVD, for decoding peripheral registers (loaded once).
    let svd = cfg.svd_path.as_ref().and_then(|p| {
        peripheral::load_svd(p)
            .map_err(|e| eprintln!("agent: {e}; peripheral decode disabled"))
            .ok()
    });

    // The RTT scan needs an awake window; retry until the control block is found.
    let mut telemetry = loop {
        match Telemetry::attach(probe.session_mut()) {
            Ok(t) => break t,
            Err(e) => {
                eprintln!("agent: attach RTT: {e}; retrying...");
                thread::sleep(Duration::from_millis(300));
            }
        }
    };
    eprintln!("agent: RTT attached; streaming telemetry (no halt)");

    let mut recent_log = RecentLog::new(RECENT_LOG_LINES);
    let mut last_crash: Option<Value> = None;
    let mut last_snapshot: Option<Value> = None;
    let mut link_errors = 0u32;
    // History: a rolling time-series of system state, for the dashboard to
    // animate ("the system over time", not just one frame).
    let mut history: VecDeque<Value> = VecDeque::new();
    let record_start = Instant::now();
    let mut last_record = record_start;
    let mut line = String::new();
    let mut buf = [0u8; 1024];

    loop {
        // 1. Drain telemetry fully this cycle. A crash is signalled by the
        //    AXYR_CRASH line; the heavy work happens after the drain.
        let mut crash: Option<Crash> = None;
        let mut read_failed = false;
        loop {
            match telemetry.read(probe.session_mut(), &mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    for &b in &buf[..n] {
                        match b {
                            b'\n' => {
                                if let Some(c) = process_line(line.trim(), &mut recent_log, &threads) {
                                    crash = Some(c);
                                }
                                line.clear();
                            }
                            b'\r' => {}
                            _ => line.push(b as char),
                        }
                    }
                }
                Err(_) => {
                    read_failed = true;
                    break;
                }
            }
        }

        // Self-heal a wedged SWD link: after a few failed cycles, re-open the
        // probe and re-attach RTT — no human needed.
        if read_failed {
            link_errors += 1;
            if link_errors >= 3 {
                eprintln!("agent: SWD link errors — reattaching...");
                if probe.reattach().is_ok()
                    && let Ok(t) = Telemetry::attach(probe.session_mut())
                {
                    telemetry = t;
                    eprintln!("agent: link recovered");
                }
                link_errors = 0;
            }
        } else {
            link_errors = 0;
        }

        // 2. On a crash, read the coredump from RAM over SWD and write the report.
        if let Some(crash) = crash {
            last_crash = report_crash(&mut probe, &cfg, coredump_addr, &crash, &recent_log);
        }

        // 3. Run any queued actions, interleaved between drains. RTT keeps
        //    buffering on the target while we do, so nothing is lost.
        while let Ok(cmd) = commands.try_recv() {
            let result = match cmd.action {
                Action::GetSnapshot => {
                    match build_snapshot(&mut probe, &cfg, svd.as_ref(), &threads, last_crash.as_ref()) {
                        Ok(v) => {
                            let text = serde_json::to_string_pretty(&v)
                                .map_err(|e| format!("serialize snapshot: {e}"));
                            last_snapshot = Some(v);
                            text
                        }
                        Err(e) => Err(e),
                    }
                }
                Action::DiffSnapshot => {
                    match build_snapshot(&mut probe, &cfg, svd.as_ref(), &threads, last_crash.as_ref()) {
                        Ok(v) => {
                            let text = match &last_snapshot {
                                Some(prev) => diff::diff(prev, &v),
                                None => "No previous snapshot; captured a baseline.".to_string(),
                            };
                            last_snapshot = Some(v);
                            Ok(text)
                        }
                        Err(e) => Err(e),
                    }
                }
                Action::GetHealth => {
                    build_health(&mut probe, svd.as_ref(), &threads, last_crash.as_ref())
                }
                Action::GetHistory => {
                    serde_json::to_string_pretty(&Value::Array(history.iter().cloned().collect()))
                        .map_err(|e| format!("serialize history: {e}"))
                }
                other => execute(&mut probe, other, svd.as_ref(), &cfg.elf),
            };
            let _ = cmd.reply.send(result);
        }

        // Record a history tick at the configured cadence.
        if last_record.elapsed() >= RECORD_INTERVAL {
            let t_ms = record_start.elapsed().as_millis() as u64;
            let tick = capture_tick(&mut probe, &cfg, &threads, t_ms);
            if history.len() >= HISTORY_TICKS {
                history.pop_front();
            }
            history.push_back(tick);
            last_record = Instant::now();
        }

        thread::sleep(Duration::from_millis(10));
    }
}

/// Record a telemetry line (recent log + thread state) and, if it is the crash
/// packet, return the parsed crash.
fn process_line(
    line: &str,
    recent_log: &mut RecentLog,
    threads: &Arc<Mutex<ThreadTable>>,
) -> Option<Crash> {
    recent_log.record(line);
    if let Ok(mut t) = threads.lock() {
        t.feed(line);
    }
    parse_crash_line(line)
}

/// Build and write the crash report: decode cause + location, read the coredump
/// from RAM over SWD and unwind it, and attach the recent telemetry.
fn report_crash(
    probe: &mut Probe,
    cfg: &Config,
    coredump_addr: Option<u64>,
    crash: &Crash,
    recent_log: &RecentLog,
) -> Option<Value> {
    let location = symbolize(&cfg.elf, &crash.pc);
    let mut report = format_report(crash, &location);

    // Read the in-memory coredump (core is halted after the fault) and unwind it.
    let mut backtrace: Option<String> = None;
    if let (Some(addr), Some(tools)) = (coredump_addr, cfg.coredump.as_ref()) {
        match probe.read_in_memory_coredump(addr) {
            Ok(Some(dump)) => match resolve_backtrace_from_bin(tools, &dump) {
                Ok(bt) => {
                    report.push_str("\nCall stack:\n");
                    report.push_str(&bt);
                    backtrace = Some(bt);
                }
                Err(e) => eprintln!("agent: backtrace: {e}"),
            },
            Ok(None) => eprintln!("agent: no valid coredump in RAM"),
            Err(e) => eprintln!("agent: read coredump: {e}"),
        }
    }

    let telemetry = recent_log.snapshot();
    if !telemetry.is_empty() {
        report.push_str("\nRecent telemetry:\n");
        report.push_str(&telemetry);
    }

    println!("=== AXYR crash report ===\n{report}");
    if let Err(e) = std::fs::write(&cfg.crash_file, report.as_bytes()) {
        eprintln!("agent: could not write crash file: {e}");
    }

    // Structured form for the snapshot contract.
    Some(json!({
        "cause": crash.reason.map(reason_label).unwrap_or("Unknown reason"),
        "reason_code": crash.reason,
        "fault_address": fault_address(&telemetry),
        "location": parse_location(&location),
        "call_stack": backtrace.as_deref().map(parse_frames).unwrap_or_default(),
        "registers": { "pc": crash.pc },
        "recent_telemetry": telemetry.lines().collect::<Vec<_>>(),
    }))
}

/// Parse "function at file:line" into {function, file, line}.
fn parse_location(s: &str) -> Value {
    let (func, loc) = s.split_once(" at ").unwrap_or((s, ""));
    let (file, line) = loc.rsplit_once(':').unwrap_or((loc, ""));
    json!({ "function": func.trim(), "file": file.trim(), "line": line.trim().parse::<u32>().ok() })
}

/// Parse "#N function at file:line" backtrace lines into frame objects.
fn parse_frames(bt: &str) -> Vec<Value> {
    bt.lines()
        .filter_map(|l| {
            let l = l.trim();
            let frame: u32 = l.strip_prefix('#')?.split_whitespace().next()?.parse().ok()?;
            let rest = l.split_once(' ')?.1;
            let mut loc = parse_location(rest);
            loc["frame"] = json!(frame);
            Some(loc)
        })
        .collect()
}

/// Extract the BFAR fault address from the Zephyr fault dump in the telemetry.
fn fault_address(telemetry: &str) -> Option<String> {
    let line = telemetry.lines().find(|l| l.contains("BFAR Address:"))?;
    let hex = line.split_whitespace().find(|t| t.starts_with("0x"))?;
    Some(hex.to_string())
}

/// Assemble the full system snapshot (the data contract) as pretty JSON. Every
/// section carries meaning, not raw bytes: device identity, decoded state
/// (reset reason / clocks), the hardware map with categories, live threads,
/// the context-switch timeline, watched variables, decoded peripherals, and the
/// last crash post-mortem.
fn build_snapshot(
    probe: &mut Probe,
    cfg: &Config,
    svd: Option<&svd_parser::svd::Device>,
    threads: &Arc<Mutex<ThreadTable>>,
    last_crash: Option<&Value>,
) -> Result<Value, String> {
    let cpuid = probe.read_word(0xE000_ED00).ok().map(|v| format!("{v:#010x}"));
    let dts = cfg.dts_path.as_ref().and_then(|p| std::fs::read_to_string(p).ok());
    let board = dts.as_deref().and_then(system_map::model);

    // Core run state.
    let core = match probe.status() {
        Ok(s) if s.contains("Running") => "running",
        Ok(s) if s.contains("Sleeping") => "sleeping",
        Ok(s) if s.contains("Halted") => {
            if last_crash.is_some() { "crashed" } else { "halted" }
        }
        _ => "unknown",
    };

    // Read RCC once: derive reset reason + clocks, and include it as a decoded
    // peripheral. All meaning comes from the SVD field names.
    let rcc = svd.and_then(|d| {
        let base = d
            .peripherals
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case("RCC"))
            .map(|p| p.base_address)?;
        let regs = peripheral::read_state(d, "RCC", |a| probe.read_word(a)).ok()?;
        Some((base, regs))
    });
    let reg_fields = |name: &str, pred: &dyn Fn(&str) -> bool| -> Option<String> {
        let (_, regs) = rcc.as_ref()?;
        let r = regs.iter().find(|r| r.name == name)?;
        let s = r
            .fields
            .iter()
            .filter(|f| pred(&f.name))
            .map(|f| f.name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        (!s.is_empty()).then_some(s)
    };
    let reset_reason = reg_fields("CSR", &|n| n.ends_with("RSTF"));
    let clocks = reg_fields("CR", &|n| n.ends_with("RDY") || n.ends_with("ON"));
    let peripherals = match rcc.as_ref() {
        Some((base, regs)) => json!([peripheral::to_json("RCC", *base, regs)]),
        None => json!([]),
    };

    let system_map = dts
        .as_deref()
        .and_then(system_map::to_json)
        .unwrap_or_else(|| json!({ "devices": [], "disabled": [] }));
    let threads_json = threads.lock().map(|t| t.to_json()).unwrap_or_else(|_| json!([]));
    let timeline = read_timeline(probe, &cfg.elf).unwrap_or_else(|| json!([]));
    let variables = read_watch(probe, &cfg.elf, &cfg.watch);

    let snapshot = json!({
        "schema_version": "1.0",
        "device": {
            "board": board, "chip": cfg.chip, "core": "ARM Cortex-M4F",
            "cpuid": cpuid, "probe": "ST-LINK/V2.1", "transport": "RTT",
        },
        "state": {
            "core": core,
            "summary": last_crash.and_then(|c| c.get("cause").cloned()),
            "reset_reason": reset_reason,
            "clocks": clocks,
        },
        "system_map": system_map,
        "threads": threads_json,
        "timeline": timeline,
        "variables": variables,
        "peripherals": peripherals,
        "crash": last_crash.cloned().unwrap_or(Value::Null),
        "actions": actions_json(),
    });
    Ok(snapshot)
}

/// Capture one history tick: the dynamic system state at time `t_ms`. Light
/// enough to record every second (threads + watched variables + core state);
/// the dashboard replays these to animate the system over time.
fn capture_tick(
    probe: &mut Probe,
    cfg: &Config,
    threads: &Arc<Mutex<ThreadTable>>,
    t_ms: u64,
) -> Value {
    let threads_json = threads.lock().map(|t| t.to_json()).unwrap_or_else(|_| json!([]));
    let variables = read_watch(probe, &cfg.elf, &cfg.watch);
    let core = match probe.status() {
        Ok(s) if s.contains("Running") => "running",
        Ok(s) if s.contains("Sleeping") => "sleeping",
        Ok(s) if s.contains("Halted") => "halted",
        _ => "unknown",
    };
    json!({ "t_ms": t_ms, "core": core, "threads": threads_json, "variables": variables })
}

/// Run proactive health checks over the captured state.
fn build_health(
    probe: &mut Probe,
    svd: Option<&svd_parser::svd::Device>,
    threads: &Arc<Mutex<ThreadTable>>,
    last_crash: Option<&Value>,
) -> Result<String, String> {
    let infos = threads.lock().map(|t| t.infos()).unwrap_or_default();
    let reset = reset_reason(probe, svd);
    let cause = last_crash
        .and_then(|c| c.get("cause").and_then(|v| v.as_str()))
        .map(String::from);
    let findings = health::analyze(&infos, reset.as_deref(), cause.as_deref());
    Ok(health::render(&findings))
}

/// The reset reason from RCC CSR (the set `*RSTF` flags), via the SVD.
fn reset_reason(probe: &mut Probe, svd: Option<&svd_parser::svd::Device>) -> Option<String> {
    let device = svd?;
    let regs = peripheral::read_state(device, "RCC", |a| probe.read_word(a)).ok()?;
    let csr = regs.iter().find(|r| r.name == "CSR")?;
    let flags = csr
        .fields
        .iter()
        .filter(|f| f.name.ends_with("RSTF"))
        .map(|f| f.name.clone())
        .collect::<Vec<_>>()
        .join(", ");
    (!flags.is_empty()).then_some(flags)
}

/// Read the context-switch timeline as JSON `[{cycles, thread}]`, oldest first.
fn read_timeline(probe: &mut Probe, elf: &str) -> Option<Value> {
    let head = symbols::resolve(elf, "axyr_trace_head").ok()?;
    let ring = symbols::resolve(elf, "axyr_trace").ok()?;
    let head_val = probe.read_word(head.address).ok()?;
    let mut raw = vec![0u32; trace::RING_SLOTS * 2];
    probe.read_words(ring.address, &mut raw).ok()?;
    let idxs = trace::ordered_indices(head_val);
    let t0 = idxs.first().map(|&i| raw[i * 2]).unwrap_or(0);
    let mut arr = Vec::new();
    for i in idxs {
        let ts = raw[i * 2];
        let name_ptr = raw[i * 2 + 1] as u64;
        let thread = if (0x2000_0000..0x2002_0000).contains(&name_ptr) {
            probe.read_cstring(name_ptr, 24).unwrap_or_default()
        } else {
            String::new()
        };
        arr.push(json!({ "cycles": ts.wrapping_sub(t0), "thread": thread }));
    }
    Some(Value::Array(arr))
}

/// Read the configured watch list as JSON `[{name, address, value}]`.
fn read_watch(probe: &mut Probe, elf: &str, watch: &[String]) -> Value {
    let mut arr = Vec::new();
    for name in watch {
        if let Ok(sym) = symbols::resolve(elf, name)
            && let Ok(value) = probe.read_word(sym.address)
        {
            arr.push(json!({
                "name": name,
                "address": format!("{:#010x}", sym.address),
                "type": "uint32",
                "value": value,
            }));
        }
    }
    Value::Array(arr)
}

/// The action catalogue, for the snapshot's `actions` section.
fn actions_json() -> Value {
    json!([
        { "name": "get_last_crash", "kind": "read" },
        { "name": "get_system_map", "kind": "read" },
        { "name": "get_threads", "kind": "read" },
        { "name": "get_trace", "kind": "read" },
        { "name": "get_snapshot", "kind": "read" },
        { "name": "read_variable", "kind": "read", "args": ["name"] },
        { "name": "read_peripheral", "kind": "read", "args": ["name"] },
        { "name": "read_memory", "kind": "read", "args": ["address", "count"] },
        { "name": "reboot_board", "kind": "action" },
        { "name": "flash_firmware", "kind": "action", "args": ["path"] }
    ])
}

/// Execute one action on the owned probe.
fn execute(
    probe: &mut Probe,
    action: Action,
    svd: Option<&svd_parser::svd::Device>,
    elf: &str,
) -> Result<String, String> {
    match action {
        Action::Reboot => {
            probe.reset()?;
            Ok("Board reset and running.".to_string())
        }
        Action::Flash(path) => {
            probe.flash_elf(&path)?;
            Ok(format!("Flashed {}; board running.", path.display()))
        }
        Action::ReadMemory { address, count } => {
            let mut words = vec![0u32; count];
            probe.read_words(address, &mut words)?;
            let mut out = String::new();
            for (i, w) in words.iter().enumerate() {
                out.push_str(&format!("{:#010x}: {w:#010x}\n", address + (i as u64) * 4));
            }
            Ok(out.trim_end().to_string())
        }
        Action::ListPeripherals => {
            let device = svd.ok_or("peripheral list not configured (set AXYR_SVD)")?;
            let list = peripheral::list(device);
            let mut out = format!("{} peripherals:\n", list.len());
            for (name, base, desc) in &list {
                let d = desc.as_deref().map(|s| format!("  — {s}")).unwrap_or_default();
                out.push_str(&format!("  {:<14} @{base:#010x}{d}\n", name));
            }
            Ok(out.trim_end().to_string())
        }
        Action::ReadPeripheral { name } => {
            let device = svd.ok_or("peripheral decode not configured (set AXYR_SVD)")?;
            peripheral::decode(device, &name, |addr| probe.read_word(addr))
        }
        Action::ListVariables => {
            let globals = symbols::list_globals(elf)?;
            if globals.is_empty() {
                return Ok("No global variables found.".to_string());
            }
            let mut out = format!("{} global variables:\n", globals.len());
            for g in &globals {
                out.push_str(&format!("  {:<32} @{:#010x}  {} bytes\n", g.name, g.address, g.size));
            }
            Ok(out.trim_end().to_string())
        }
        Action::ReadVariable { name } => {
            // Resolve the global from the ELF, then read it live over SWD. A
            // background read works while the core runs or sleeps — no halt,
            // non-intrusive.
            let sym = symbols::resolve(elf, &name)?;
            let words = (sym.size.max(1) as usize).div_ceil(4).clamp(1, 64);
            let mut buf = vec![0u32; words];
            probe.read_words(sym.address, &mut buf)?;
            let mut out = format!("{} @{:#010x} ({} bytes) =", sym.name, sym.address, sym.size);
            if sym.size <= 4 {
                out.push_str(&format!(" {:#010x} ({})", buf[0], buf[0]));
            } else {
                for w in &buf {
                    out.push_str(&format!(" {w:#010x}"));
                }
            }
            Ok(out)
        }
        // GetSnapshot is routed to build_snapshot by the owner loop (it needs
        // more context than execute carries); never reaches here.
        Action::GetSnapshot => Err("internal: snapshot handled elsewhere".to_string()),
        Action::GetHealth => Err("internal: health handled elsewhere".to_string()),
        Action::DiffSnapshot => Err("internal: diff handled elsewhere".to_string()),
        Action::GetHistory => Err("internal: history handled elsewhere".to_string()),
        Action::WatchUntil { name, value } => {
            // Non-intrusive trigger: poll the global with background reads (no
            // halt) until it reaches the value, bounded by a timeout. A true
            // hardware watchpoint would halt the core (intrusive) without SWO.
            let sym = symbols::resolve(elf, &name)?;
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let v = probe.read_word(sym.address)?;
                if v == value {
                    return Ok(format!("Triggered: {name} = {value} ({value:#x})"));
                }
                if Instant::now() >= deadline {
                    return Ok(format!("Timeout (10s): {name} = {v} ({v:#x}), waited for {value}"));
                }
                thread::sleep(Duration::from_millis(20));
            }
        }
        Action::ReadTrace => {
            let head = symbols::resolve(elf, "axyr_trace_head")?;
            let ring = symbols::resolve(elf, "axyr_trace")?;
            let head_val = probe.read_word(head.address)?;
            let mut raw = vec![0u32; trace::RING_SLOTS * 2];
            probe.read_words(ring.address, &mut raw)?;
            let mut entries = Vec::new();
            for idx in trace::ordered_indices(head_val) {
                let ts = raw[idx * 2];
                let name_ptr = raw[idx * 2 + 1] as u64;
                // Names live in RAM; read the string, else show the raw pointer.
                let thread = if (0x2000_0000..0x2002_0000).contains(&name_ptr) {
                    probe
                        .read_cstring(name_ptr, 24)
                        .unwrap_or_else(|_| format!("{name_ptr:#010x}"))
                } else {
                    format!("{name_ptr:#010x}")
                };
                entries.push(trace::TraceEntry { timestamp: ts, thread });
            }
            Ok(trace::format_timeline(&entries))
        }
    }
}
