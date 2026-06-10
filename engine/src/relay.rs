//! The agent's relay loop: serve the local probe to a remote engine.
//!
//! This is the **dumb** half of the split. It drains RTT locally and pushes the
//! bytes to the engine, and it answers the engine's raw primitive requests
//! (read words, reset, flash bytes). No ELF, no DWARF, no devicetree, no SVD —
//! nothing here understands the firmware, so there is nothing here worth
//! reverse-engineering. All of that lives in the engine.

use std::io::ErrorKind;
use std::net::TcpStream;
use std::thread;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use serde_json::{Value, json};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

use crate::link::{LocalLink, ProbeLink};
use crate::wire::{self, AgentFrame, Hello, Op, Request};

/// How often the relay drains RTT (and thus how fresh pushed telemetry is).
const POLL: Duration = Duration::from_millis(10);
/// Reconnect backoff when the engine is unreachable or the link drops.
const RECONNECT: Duration = Duration::from_secs(2);

/// Connect to the engine and serve the probe forever: reconnects with backoff
/// on any link loss, re-sending `hello` each time (a reconnect is a new
/// engine-side session).
pub fn run(mut link: LocalLink, hello: &Hello, url: &str) -> ! {
    loop {
        match connect(url) {
            Ok(mut ws) => {
                eprintln!("axyr: connected to engine at {url}");
                if ws.send(Message::Text(wire::encode(hello).into())).is_ok() {
                    serve(&mut link, &mut ws);
                }
                eprintln!("axyr: engine link lost; reconnecting in {RECONNECT:?} ...");
            }
            Err(e) => {
                eprintln!("axyr: connect {url}: {e}; retrying in {RECONNECT:?} ...");
            }
        }
        thread::sleep(RECONNECT);
    }
}

fn connect(url: &str) -> Result<WebSocket<MaybeTlsStream<TcpStream>>, String> {
    let (ws, _) = tungstenite::connect(url).map_err(|e| e.to_string())?;
    // A short read timeout lets the loop interleave telemetry pushes with
    // request handling on one thread.
    let stream = match ws.get_ref() {
        MaybeTlsStream::Plain(s) => s,
        MaybeTlsStream::Rustls(t) => t.get_ref(),
        _ => return Err("unsupported stream type".to_string()),
    };
    stream
        .set_read_timeout(Some(POLL))
        .map_err(|e| format!("set read timeout: {e}"))?;
    Ok(ws)
}

/// One connected session: push telemetry, answer primitives, until the link dies.
fn serve(link: &mut LocalLink, ws: &mut WebSocket<MaybeTlsStream<TcpStream>>) {
    let mut chunk: Vec<u8> = Vec::new();
    loop {
        // Push whatever RTT produced since the last cycle (self-heals inside).
        chunk.clear();
        let _ = link.poll_telemetry(&mut chunk);
        if !chunk.is_empty() {
            let frame = AgentFrame::Telemetry { data_b64: B64.encode(&chunk) };
            if ws.send(Message::Text(wire::encode(&frame).into())).is_err() {
                return;
            }
        }

        // Serve at most a few requests per cycle, then poll telemetry again.
        match ws.read() {
            Ok(Message::Text(text)) => {
                let frame = match wire::decode::<Request>(text.as_str()) {
                    Ok(req) => execute(link, req),
                    Err(e) => {
                        eprintln!("axyr: bad request from engine: {e}");
                        continue;
                    }
                };
                if ws.send(Message::Text(wire::encode(&frame).into())).is_err() {
                    return;
                }
            }
            Ok(Message::Close(_)) => return,
            Ok(_) => {} // ping/pong handled by tungstenite; binary unused
            Err(tungstenite::Error::Io(e))
                if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {}
            Err(e) => {
                eprintln!("axyr: engine socket error: {e}");
                return;
            }
        }
    }
}

/// Run one primitive on the local probe and wrap the result for the wire.
fn execute(link: &mut LocalLink, req: Request) -> AgentFrame {
    let result: Result<Value, String> = match req.op {
        Op::ReadWord { addr } => link.read_word(addr).map(|v| json!(v)),
        Op::ReadWords { addr, count } => {
            // Cap pathological sizes; the engine never asks for more than a
            // trace ring (~hundreds of words).
            let count = count.min(65536);
            let mut words = vec![0u32; count];
            link.read_words(addr, &mut words).map(|()| json!(words))
        }
        Op::ReadCstring { addr, max } => link.read_cstring(addr, max.min(4096)).map(|s| json!(s)),
        Op::ReadCoredump { base } => link.read_coredump(base).map(|d| match d {
            Some(bytes) => json!({ "data_b64": B64.encode(bytes) }),
            None => json!({}),
        }),
        Op::Status => link.status().map(|s| json!(s)),
        Op::Reset => link.reset().map(|()| Value::Null),
        Op::Flash { elf_b64 } => B64
            .decode(elf_b64.as_bytes())
            .map_err(|e| format!("bad flash payload: {e}"))
            .and_then(|elf| link.flash(&elf))
            .map(|()| Value::Null),
        Op::ResyncTelemetry => {
            link.resync_telemetry();
            Ok(Value::Null)
        }
    };
    match result {
        Ok(v) => AgentFrame::Reply { id: req.id, ok: Some(v), err: None },
        Err(e) => AgentFrame::Reply { id: req.id, ok: None, err: Some(e) },
    }
}
