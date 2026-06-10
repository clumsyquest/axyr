//! The engine's front-ends: the HTTP API for the dashboard and MCP over
//! stdio for AI agents. Both route through the SAME [`dispatch_tool`], so the
//! web UI and an MCP agent always see one truth. Shared by the all-local
//! `axyr` mode and the `axyr-engined` server.

use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use tiny_http::{Header, Method, ReadWrite, Response, Server};

use crate::agent::{Action, Command};
use crate::system_map;
use crate::threads::ThreadTable;

/// Everything the HTTP front-end needs, swappable behind locks so a server
/// (`axyr-engined`) can replace the live session when an agent reconnects —
/// the all-local mode just fills it once and never touches it again.
pub struct Shared {
    pub crash_file: Mutex<String>,
    pub dts: Mutex<Option<String>>,
    pub firmware_dir: Mutex<Option<String>>,
    pub cmd_tx: Mutex<Sender<Command>>,
    pub connect: Mutex<String>,
    pub threads: Arc<Mutex<ThreadTable>>,
}

/// Handler for an agent that connected via the `/agent` WebSocket upgrade —
/// receives the raw upgraded stream (the HTTP 101 is already sent). Lets the
/// engine ride agents over the SAME port as the dashboard API, so a single
/// HTTPS edge (e.g. Railway's) carries both, and agents get TLS for free
/// (`wss://engine-host/agent`).
pub type AgentUpgrade = Box<dyn Fn(Box<dyn ReadWrite + Send>) + Send + Sync>;

/// MCP over stdio: one JSON-RPC message per line.
pub fn serve_mcp(
    crash_file: &str,
    dts: Option<&str>,
    firmware_dir: Option<&str>,
    cmd_tx: &Sender<Command>,
    threads: &Arc<Mutex<ThreadTable>>,
) {
    let stdin = io::stdin();
    let stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line.expect("failed to read line");
        if line.trim().is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");
        let id = req.get("id").cloned();

        let response: Option<Value> = match method {
            "initialize" => {
                let version = req
                    .pointer("/params/protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or("2024-11-05");
                Some(json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": {
                        "protocolVersion": version,
                        "capabilities": { "tools": {} },
                        "serverInfo": { "name": "axyr-engine", "version": "0.1.0" }
                    }
                }))
            }
            "tools/list" => Some(json!({
                "jsonrpc": "2.0", "id": id, "result": { "tools": tool_definitions() }
            })),
            "tools/call" => {
                let tool = req.pointer("/params/name").and_then(Value::as_str).unwrap_or("");
                let args = req.pointer("/params/arguments").cloned().unwrap_or(json!({}));
                let result =
                    dispatch_tool(tool, &args, crash_file, dts, firmware_dir, cmd_tx, threads);
                Some(match result {
                    Ok(text) => json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": { "content": [{ "type": "text", "text": text }] }
                    }),
                    Err(text) => json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": { "content": [{ "type": "text", "text": text }], "isError": true }
                    }),
                })
            }
            _ => None, // notifications: no reply
        };

        if let Some(resp) = response {
            let mut out = stdout.lock();
            writeln!(out, "{resp}").expect("write failed");
            out.flush().expect("flush failed");
        }
    }
}

fn tool_definitions() -> Value {
    json!([
        {
            "name": "get_last_crash",
            "description": "Return the most recent crash captured from the board (cause, source location, call stack, and recent telemetry).",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "get_system_map",
            "description": "Return the board's hardware map from the Zephyr devicetree: peripherals (I2C/SPI/UART/timers/GPIO) and the sensors/actuators on them, with addresses and on/off state.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "get_graph",
            "description": "Return the system as a connected graph (board root + nodes with kind/address/status + parent->child edges like soc->i2c1->bme280), ready for a schematic/diagram view.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "get_threads",
            "description": "Return the live RTOS thread state: per-thread stack usage and CPU load (from the firmware's thread analyzer over RTT).",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "reboot_board",
            "description": "Reset the target board over the debug probe and let it run.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "watch_until",
            "description": "Non-intrusive trigger: poll a global variable until it reaches a value (or 10s timeout), without halting the core. Useful to wait for a condition before inspecting.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Global variable name." },
                    "value": { "type": "integer", "description": "Value to wait for." }
                },
                "required": ["name", "value"]
            }
        },
        {
            "name": "list_firmware",
            "description": "List flashable firmware images (.elf/.bin) available on the host — so you can pick a path for flash_firmware without a human.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "flash_firmware",
            "description": "Flash an ELF image onto the target board's flash, then leave it running.",
            "inputSchema": {
                "type": "object",
                "properties": { "path": { "type": "string", "description": "Absolute path to the ELF file to flash." } },
                "required": ["path"]
            }
        },
        {
            "name": "get_trace",
            "description": "Return the context-switch timeline ('what ran when'): recent thread switches with cycle timestamps, read from a RAM ring buffer.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "get_history",
            "description": "Return the recorded time-series of system state (threads, watched variables, core state per tick) — the data to animate the system over time, not just one frame.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "diff_snapshot",
            "description": "Take a fresh snapshot and report what changed since the previous one (values, registers, state, threads) — 'what moved since it was working'.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "get_health",
            "description": "Run proactive health checks and report anomalies (stack near overflow, runaway/starved thread, watchdog/brown-out reset, active crash) — insight, not raw data.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "get_snapshot",
            "description": "Return the whole system in one structured JSON snapshot (the data contract): device, state (reset reason/clocks), hardware map, threads, timeline, watched variables, decoded peripherals, and the last crash. Use this to understand and map the real system state.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "list_variables",
            "description": "List the firmware's global variables (name, address, size), discovered from the ELF — so you can see what the project exposes and choose what to read.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "read_variable",
            "description": "Read a firmware global variable live by name: resolves its address from the ELF and reads it over SWD (works while the core runs or sleeps, no halt).",
            "inputSchema": {
                "type": "object",
                "properties": { "name": { "type": "string", "description": "The global variable's symbol name, e.g. \"axyr_counter\"." } },
                "required": ["name"]
            }
        },
        {
            "name": "list_peripherals",
            "description": "List the chip's peripherals (name, address, description) from the SVD — so you can see what to pass to read_peripheral.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "read_peripheral",
            "description": "Decode a peripheral's live register state in plain language (e.g. GPIOA, USART2, RCC) using the chip's SVD: each readable register and its bit-fields with their meaning.",
            "inputSchema": {
                "type": "object",
                "properties": { "name": { "type": "string", "description": "Peripheral name, e.g. \"GPIOA\" or \"USART2\"." } },
                "required": ["name"]
            }
        },
        {
            "name": "read_memory",
            "description": "Read 32-bit words from the target's memory over SWD. Useful to inspect registers or peripherals.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "address": { "type": "string", "description": "Start address, e.g. \"0xE000ED00\" or decimal." },
                    "count": { "type": "integer", "description": "Number of 32-bit words to read (default 1, max 256)." }
                },
                "required": ["address"]
            }
        }
    ])
}

/// Route a tool call: reads are served here; actions go to the probe-owner
/// thread and we block on its reply.
pub fn dispatch_tool(
    tool: &str,
    args: &Value,
    crash_file: &str,
    dts: Option<&str>,
    firmware_dir: Option<&str>,
    cmd_tx: &Sender<Command>,
    threads: &Arc<Mutex<ThreadTable>>,
) -> Result<String, String> {
    match tool {
        "list_firmware" => list_firmware(firmware_dir),
        "watch_until" => {
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .ok_or("missing required argument: name")?;
            let value = args
                .get("value")
                .and_then(Value::as_u64)
                .ok_or("missing required argument: value")? as u32;
            run_action(cmd_tx, Action::WatchUntil { name: name.to_string(), value })
        }
        "get_last_crash" => Ok(fs::read_to_string(crash_file)
            .unwrap_or_else(|_| "No crash recorded yet.".to_string())),
        "get_threads" => Ok(threads
            .lock()
            .map_err(|_| "thread state lock poisoned".to_string())?
            .render()),
        "get_system_map" => {
            let path = dts.ok_or("system map not configured (set AXYR_DTS)")?;
            let src = fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
            system_map::render(&src).ok_or_else(|| "could not parse devicetree".to_string())
        }
        "get_graph" => {
            let path = dts.ok_or("graph not configured (set AXYR_DTS)")?;
            let src = fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
            system_map::graph_json(&src)
                .map(|v| v.to_string())
                .ok_or_else(|| "could not parse devicetree".to_string())
        }
        "get_trace" => run_action(cmd_tx, Action::ReadTrace),
        "get_snapshot" => run_action(cmd_tx, Action::GetSnapshot),
        "get_health" => run_action(cmd_tx, Action::GetHealth),
        "diff_snapshot" => run_action(cmd_tx, Action::DiffSnapshot),
        "get_history" => run_action(cmd_tx, Action::GetHistory),
        "list_variables" => run_action(cmd_tx, Action::ListVariables),
        "list_peripherals" => run_action(cmd_tx, Action::ListPeripherals),
        "read_variable" => {
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .ok_or("missing required argument: name")?;
            run_action(cmd_tx, Action::ReadVariable { name: name.to_string() })
        }
        "read_peripheral" => {
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .ok_or("missing required argument: name")?;
            run_action(cmd_tx, Action::ReadPeripheral { name: name.to_string() })
        }
        "reboot_board" => run_action(cmd_tx, Action::Reboot),
        "flash_firmware" => {
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .ok_or("missing required argument: path")?;
            run_action(cmd_tx, Action::Flash(PathBuf::from(path)))
        }
        "read_memory" => {
            let addr_str = args
                .get("address")
                .and_then(Value::as_str)
                .ok_or("missing required argument: address")?;
            let address = parse_address(addr_str)?;
            let count = args
                .get("count")
                .and_then(Value::as_u64)
                .unwrap_or(1)
                .clamp(1, 256) as usize;
            run_action(cmd_tx, Action::ReadMemory { address, count })
        }
        other => Err(format!("Unknown tool: {other}")),
    }
}

/// Send an action to the owner thread and wait for its result.
fn run_action(cmd_tx: &Sender<Command>, action: Action) -> Result<String, String> {
    let (reply, rx) = mpsc::channel();
    cmd_tx
        .send(Command { action, reply })
        .map_err(|_| "agent thread is gone".to_string())?;
    rx.recv().map_err(|_| "no reply from agent thread".to_string())?
}

/// List flashable firmware images (.elf/.bin) in the configured directory, so an
/// autonomous agent can choose a path for `flash_firmware`.
fn list_firmware(dir: Option<&str>) -> Result<String, String> {
    let dir = dir.ok_or("no firmware directory (set AXYR_FIRMWARE_DIR)")?;
    let mut images: Vec<(String, u64)> = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| format!("read dir {dir}: {e}"))? {
        let entry = entry.map_err(|e| format!("dir entry: {e}"))?;
        let path = entry.path();
        if matches!(path.extension().and_then(|e| e.to_str()), Some("elf") | Some("bin")) {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            images.push((path.display().to_string(), size));
        }
    }
    images.sort();
    if images.is_empty() {
        return Ok(format!("No firmware images (.elf/.bin) in {dir}"));
    }
    let mut out = format!("Flashable firmware in {dir}:\n");
    for (path, size) in images {
        out.push_str(&format!("  {path}  ({size} bytes)\n"));
    }
    Ok(out.trim_end().to_string())
}

/// Parse a memory address given as hex ("0x...") or decimal.
fn parse_address(s: &str) -> Result<u64, String> {
    let s = s.trim();
    let parsed = if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16)
    } else {
        s.parse::<u64>()
    };
    parsed.map_err(|_| format!("invalid address: {s:?}"))
}

/// HTTP API for the dashboard: a second front-end alongside MCP stdio. Each route
/// maps to a tool and runs through the SAME [`dispatch_tool`], so the web UI never
/// sees a different view of the system than an MCP agent does. Responses are JSON
/// for the JSON tools and plain text otherwise; CORS is permissive so a dashboard
/// served from another origin can fetch it.
pub fn serve_http(addr: &str, shared: Arc<Shared>, on_agent: Option<AgentUpgrade>) {
    let server = match Server::http(addr) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("HTTP API disabled (bind {addr}: {e})");
            return;
        }
    };

    for mut request in server.incoming_requests() {
        let method = request.method().clone();
        let url = request.url().to_string();
        let (path, query) = url.split_once('?').unwrap_or((url.as_str(), ""));
        let q = parse_query(query);

        // CORS preflight.
        if method == Method::Options {
            let _ = request.respond(
                Response::empty(204)
                    .with_header(header("Access-Control-Allow-Origin", "*"))
                    .with_header(header("Access-Control-Allow-Methods", "GET, POST, OPTIONS"))
                    .with_header(header("Access-Control-Allow-Headers", "Content-Type")),
            );
            continue;
        }

        // Index route describes the API.
        if method == Method::Get && path == "/" {
            let _ = request.respond(json_ok(&http_index()));
            continue;
        }
        // Agent WebSocket upgrade: hand the raw socket to the engine server.
        if method == Method::Get && path == "/agent" {
            match (&on_agent, websocket_accept_key(&request)) {
                (Some(handler), Some(accept)) => {
                    let response = Response::empty(101)
                        .with_header(header("Upgrade", "websocket"))
                        .with_header(header("Connection", "Upgrade"))
                        .with_header(header("Sec-WebSocket-Accept", &accept));
                    let stream = request.upgrade("websocket", response);
                    handler(stream);
                }
                (Some(_), None) => {
                    let _ = request.respond(text_response("expected a websocket upgrade".into(), 400));
                }
                (None, _) => {
                    let _ = request.respond(text_response(
                        "this axyr instance does not accept remote agents".into(),
                        404,
                    ));
                }
            }
            continue;
        }

        // Connect screen: what board/probe/build was detected (static, no probe).
        if method == Method::Get && path == "/connect" {
            let connect = shared.connect.lock().map(|c| c.clone()).unwrap_or_default();
            let _ = request.respond(json_ok(&connect));
            continue;
        }

        // POST body (for action routes).
        let body_json: Value = if method == Method::Post {
            let mut body = String::new();
            let _ = request.as_reader().read_to_string(&mut body);
            serde_json::from_str(&body).unwrap_or_else(|_| json!({}))
        } else {
            json!({})
        };

        // Map (method, path) -> (tool, args).
        let routed: Option<(&str, Value)> = match (&method, path) {
            (Method::Get, "/snapshot") => Some(("get_snapshot", json!({}))),
            (Method::Get, "/graph") => Some(("get_graph", json!({}))),
            (Method::Get, "/system_map") => Some(("get_system_map", json!({}))),
            (Method::Get, "/threads") => Some(("get_threads", json!({}))),
            (Method::Get, "/trace") => Some(("get_trace", json!({}))),
            (Method::Get, "/history") => Some(("get_history", json!({}))),
            (Method::Get, "/health") => Some(("get_health", json!({}))),
            (Method::Get, "/crash") => Some(("get_last_crash", json!({}))),
            (Method::Get, "/variables") => Some(("list_variables", json!({}))),
            (Method::Get, "/peripherals") => Some(("list_peripherals", json!({}))),
            (Method::Get, "/firmware") => Some(("list_firmware", json!({}))),
            (Method::Get, "/variable") => Some(("read_variable", json!({ "name": q.get("name") }))),
            (Method::Get, "/peripheral") => {
                Some(("read_peripheral", json!({ "name": q.get("name") })))
            }
            (Method::Get, "/memory") => Some((
                "read_memory",
                json!({
                    "address": q.get("address"),
                    "count": q.get("count").and_then(|c| c.parse::<u64>().ok()),
                }),
            )),
            (Method::Post, "/reboot") => Some(("reboot_board", json!({}))),
            (Method::Post, "/diff") => Some(("diff_snapshot", json!({}))),
            (Method::Post, "/flash") => Some(("flash_firmware", body_json.clone())),
            (Method::Post, "/watch") => Some(("watch_until", body_json.clone())),
            _ => None,
        };

        let response = match routed {
            Some((tool, args)) => {
                // Snapshot the live session under the locks, then dispatch.
                let crash_file = shared.crash_file.lock().map(|v| v.clone()).unwrap_or_default();
                let dts = shared.dts.lock().ok().and_then(|v| v.clone());
                let firmware_dir = shared.firmware_dir.lock().ok().and_then(|v| v.clone());
                let cmd_tx = shared.cmd_tx.lock().map(|v| v.clone());
                let result = match cmd_tx {
                    Ok(cmd_tx) => dispatch_tool(
                        tool, &args, &crash_file, dts.as_deref(), firmware_dir.as_deref(),
                        &cmd_tx, &shared.threads,
                    ),
                    Err(_) => Err("engine state lock poisoned".to_string()),
                };
                match result {
                    Ok(text) => json_ok(&text),
                    Err(text) => text_response(text, 400),
                }
            }
            None => text_response("not found".to_string(), 404),
        };
        let _ = request.respond(response);
    }
}

/// A 200 response, JSON content-type when the body looks like JSON, with CORS.
fn json_ok(text: &str) -> Response<io::Cursor<Vec<u8>>> {
    let trimmed = text.trim_start();
    let ct = if trimmed.starts_with('{') || trimmed.starts_with('[') {
        "application/json"
    } else {
        "text/plain; charset=utf-8"
    };
    Response::from_string(text)
        .with_header(header("Content-Type", ct))
        .with_header(header("Access-Control-Allow-Origin", "*"))
}

/// A plain-text response with the given status code, with CORS.
fn text_response(text: String, status: u16) -> Response<io::Cursor<Vec<u8>>> {
    Response::from_string(text)
        .with_status_code(status)
        .with_header(header("Content-Type", "text/plain; charset=utf-8"))
        .with_header(header("Access-Control-Allow-Origin", "*"))
}

/// RFC 6455 handshake: derive Sec-WebSocket-Accept from the client's key.
/// `None` if the request isn't a websocket upgrade.
fn websocket_accept_key(request: &tiny_http::Request) -> Option<String> {
    const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
    let key = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Sec-WebSocket-Key"))?
        .value
        .as_str()
        .trim()
        .to_string();
    let mut sha = Sha1::new();
    sha.update(key.as_bytes());
    sha.update(GUID.as_bytes());
    Some(B64.encode(sha.finalize()))
}

fn header(key: &str, value: &str) -> Header {
    Header::from_bytes(key.as_bytes(), value.as_bytes()).expect("valid header")
}

/// Parse a URL query string into a key→value map (percent-decoded).
fn parse_query(query: &str) -> HashMap<String, String> {
    query
        .split('&')
        .filter(|s| !s.is_empty())
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.to_string(), urldecode(v)))
        .collect()
}

/// Minimal percent-decoding (`+` → space, `%XX` → byte) for query values.
fn urldecode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 3 <= bytes.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(b) => {
                    out.push(b);
                    i += 3;
                }
                Err(_) => {
                    out.push(b'%');
                    i += 1;
                }
            },
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

/// JSON description of the HTTP API, served at `/`.
fn http_index() -> String {
    json!({
        "service": "axyr-engine",
        "endpoints": {
            "GET /connect": "detected board / probe / build for the Connect screen (JSON)",
            "GET /snapshot": "whole system snapshot (JSON)",
            "GET /graph": "system as nodes+edges for the schematic view (JSON)",
            "GET /system_map": "hardware map (text)",
            "GET /threads": "RTOS thread state (text)",
            "GET /trace": "context-switch timeline (text)",
            "GET /history": "recorded time-series of system state (JSON)",
            "GET /health": "anomaly checks (text)",
            "GET /crash": "last captured crash (text)",
            "GET /variables": "global variables from the ELF (text)",
            "GET /peripherals": "chip peripherals from the SVD (text)",
            "GET /firmware": "flashable images on the host (text)",
            "GET /variable?name=<sym>": "read one global live",
            "GET /peripheral?name=<NAME>": "decode one peripheral live",
            "GET /memory?address=<hex>&count=<n>": "read memory words",
            "POST /reboot": "reset the board",
            "POST /diff": "snapshot diff vs previous",
            "POST /flash {path}": "flash an ELF image",
            "POST /watch {name,value}": "non-intrusive wait for a value"
        }
    })
    .to_string()
}
