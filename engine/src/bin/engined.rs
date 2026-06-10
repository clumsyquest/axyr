//! axyr-engined — the engine as a server.
//!
//! The other half of the agent/engine split: agents (dumb relays owning a
//! probe, see `axyr --connect`) dial in over WebSocket and upload their build
//! artifacts; ALL analysis (ELF/DWARF, devicetree, SVD, crash post-mortem,
//! history) runs here; the dashboard talks to the same HTTP API as in local
//! mode — only the base URL changes. Run it on your LAN or in the cloud; the
//! engine never needs USB access, only the agent does.
//!
//! Environment:
//!   AXYR_HTTP          HTTP API bind (default 127.0.0.1:7878; PORT overrides
//!                      the port and binds 0.0.0.0 — what Railway sets)
//!   AXYR_AGENT_LISTEN  agent WebSocket bind (default 127.0.0.1:7879;
//!                      AGENT_PORT overrides port and binds 0.0.0.0)
//!   AXYR_SVD           optional SVD path for peripheral decode

use std::env;
use std::fs;
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use serde_json::json;
use tungstenite::{Message, WebSocket};

use axyr_engine::agent::{self, Config};
use axyr_engine::api::{self, Shared};
use axyr_engine::remote::{self, RemoteLink};
use axyr_engine::system_map;
use axyr_engine::threads::ThreadTable;
use axyr_engine::wire::{self, Hello};

fn main() {
    let http_addr = env::var("AXYR_HTTP").unwrap_or_else(|_| match env::var("PORT") {
        Ok(p) => format!("0.0.0.0:{p}"),
        Err(_) => "127.0.0.1:7878".to_string(),
    });
    let agent_addr = env::var("AXYR_AGENT_LISTEN").unwrap_or_else(|_| {
        match env::var("AGENT_PORT") {
            Ok(p) => format!("0.0.0.0:{p}"),
            Err(_) => "127.0.0.1:7879".to_string(),
        }
    });

    let listener = TcpListener::bind(&agent_addr).unwrap_or_else(|e| {
        eprintln!("axyr-engined: bind agent listener {agent_addr}: {e}");
        std::process::exit(1);
    });

    // The HTTP front-end starts immediately; /connect says "no agent yet"
    // until one dials in, then each new agent session swaps the state below.
    let (placeholder_tx, _placeholder_rx) = mpsc::channel();
    let shared = Arc::new(Shared {
        crash_file: Mutex::new(String::new()),
        dts: Mutex::new(None),
        firmware_dir: Mutex::new(None),
        cmd_tx: Mutex::new(placeholder_tx),
        connect: Mutex::new(json!({ "ready": false, "waiting": "no agent connected yet" }).to_string()),
        threads: Arc::new(Mutex::new(ThreadTable::new())),
    });
    {
        let http_shared = shared.clone();
        let http_addr = http_addr.clone();
        thread::spawn(move || api::serve_http(&http_addr, http_shared));
    }

    eprintln!("axyr-engined — engine ready");
    eprintln!("  dashboard API : http://{http_addr}");
    eprintln!("  agents dial   : ws://{agent_addr}");

    for (n, stream) in listener.incoming().flatten().enumerate() {
        match accept_agent(stream, n) {
            Ok((hello, ws)) => start_session(hello, ws, &shared, n),
            Err(e) => eprintln!("axyr-engined: agent handshake failed: {e}"),
        }
    }
}

/// WebSocket-accept an incoming agent and read its Hello.
fn accept_agent(stream: TcpStream, n: usize) -> Result<(Hello, WebSocket<TcpStream>), String> {
    let peer = stream.peer_addr().map(|a| a.to_string()).unwrap_or_default();
    // Generous timeout for the handshake + Hello (the ELF rides inside it),
    // tightened afterwards so the socket thread can multiplex.
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|e| format!("set timeout: {e}"))?;
    let mut ws = tungstenite::accept(stream).map_err(|e| format!("ws accept: {e}"))?;

    let hello: Hello = loop {
        match ws.read().map_err(|e| format!("read hello: {e}"))? {
            Message::Text(text) => break wire::decode(text.as_str())?,
            Message::Close(_) => return Err("agent closed before hello".to_string()),
            _ => continue,
        }
    };
    if hello.wire_version != wire::WIRE_VERSION {
        return Err(format!(
            "agent #{n} from {peer} speaks wire v{}, engine is v{} — update the agent",
            hello.wire_version,
            wire::WIRE_VERSION
        ));
    }
    eprintln!(
        "axyr-engined: agent #{n} connected from {peer} ({} v{}, chip {})",
        hello.probe.as_ref().map(|(p, _)| p.as_str()).unwrap_or("unknown probe"),
        hello.agent_version,
        hello.chip
    );
    ws.get_ref()
        .set_read_timeout(Some(Duration::from_millis(5)))
        .map_err(|e| format!("set timeout: {e}"))?;
    Ok((hello, ws))
}

/// Materialize the uploaded artifacts, wire a RemoteLink to the socket, start
/// the analysis loop, and swap this session into the HTTP front-end.
fn start_session(hello: Hello, ws: WebSocket<TcpStream>, shared: &Arc<Shared>, n: usize) {
    // Session workspace: the uploaded ELF/DTS as files, so every existing
    // analysis path (symbols, DWARF, devicetree) works unchanged.
    let dir = env::temp_dir().join(format!("axyr-engined-{}-{n}", std::process::id()));
    if let Err(e) = fs::create_dir_all(&dir) {
        eprintln!("axyr-engined: create session dir: {e}");
        return;
    }
    let elf_path = dir.join("zephyr.elf");
    let elf_bytes = match B64.decode(hello.elf_b64.as_bytes()) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("axyr-engined: bad ELF payload: {e}");
            return;
        }
    };
    if let Err(e) = fs::write(&elf_path, &elf_bytes) {
        eprintln!("axyr-engined: write session ELF: {e}");
        return;
    }
    let dts_path = hello.dts.as_ref().and_then(|content| {
        let p = dir.join("zephyr.dts");
        fs::write(&p, content).ok().map(|()| p.display().to_string())
    });
    let crash_file = dir.join("last-crash.txt").display().to_string();

    // The remote link: the socket thread owns the WebSocket, the analysis
    // loop drives the link like a local probe.
    let (link, req_rx, tel_tx) = RemoteLink::new();
    thread::spawn(move || remote::serve_socket(ws, req_rx, tel_tx));

    let cfg = Config {
        elf: elf_path.display().to_string(),
        crash_file: crash_file.clone(),
        coredump: None, // native DWARF unwinding; no external tools in the cloud
        svd_path: env::var("AXYR_SVD").ok(),
        chip: hello.chip.clone(),
        dts_path: dts_path.clone(),
        watch: hello.watch.clone(),
    };
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let threads = shared.threads.clone();
    thread::spawn(move || agent::run(Box::new(link), cfg, cmd_rx, threads));

    // Swap this session into the HTTP front-end (replaces a dead one after an
    // agent reconnect; the dashboard never has to care).
    let board = hello
        .dts
        .as_deref()
        .and_then(system_map::model);
    let connect = json!({
        "probe": hello.probe.as_ref().map(|(p, s)| json!({ "name": p, "serial": s })),
        "board": board,
        "chip": hello.chip,
        "detection": hello.detection,
        "observation_only": hello.generic,
        "chip_mismatch": false,
        "cpuid": hello.cpuid.map(|c| format!("{c:#010x}")),
        "dev_id": hello.devid.map(|c| format!("{:#05x}", c & 0xfff)),
        "build": { "elf": elf_path.display().to_string(), "dts": dts_path, "uploaded": true },
        "agent": { "version": hello.agent_version, "remote": true },
        "ready": true,
    })
    .to_string();
    let set = |field: &Mutex<String>, v: String| {
        if let Ok(mut g) = field.lock() {
            *g = v;
        }
    };
    set(&shared.crash_file, crash_file);
    set(&shared.connect, connect);
    if let Ok(mut g) = shared.dts.lock() {
        *g = dts_path;
    }
    if let Ok(mut g) = shared.firmware_dir.lock() {
        *g = Some(dir.display().to_string());
    }
    if let Ok(mut g) = shared.cmd_tx.lock() {
        *g = cmd_tx;
    }
}
