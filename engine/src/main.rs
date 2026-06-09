//! axyr-engine — the local agent.
//!
//! It owns the debug probe (the single, serial SWD resource) on one background
//! thread that drains RTT telemetry and runs board actions (see [`agent`]). The
//! main thread is the MCP front-end (stdio, one JSON-RPC message per line): it
//! serves read tools directly and routes action tools to the owner thread over a
//! channel, so an agent's actions never disturb the real-time telemetry.

use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::{Value, json};

use axyr_engine::agent::{self, Action, Command, Config};
use axyr_engine::coredump::CoredumpTools;
use axyr_engine::probe::{DEFAULT_CHIP, Probe};
use axyr_engine::threads::ThreadTable;
use axyr_engine::system_map;

fn main() {
    // Usage: axyr-engine <elf> <addr2line> <crash-file>
    let args: Vec<String> = env::args().collect();
    if args.len() != 4 {
        eprintln!("usage: {} <elf> <addr2line> <crash-file>", args[0]);
        std::process::exit(1);
    }
    let elf = args[1].clone();
    let addr2line = args[2].clone();
    let crash_file = args[3].clone();

    let chip = env::var("AXYR_CHIP").unwrap_or_else(|_| DEFAULT_CHIP.to_string());
    let dts = env::var("AXYR_DTS").ok();
    let coredump = CoredumpTools::from_env(&elf);
    if coredump.is_none() {
        eprintln!("note: coredump tools not configured; call stack disabled");
    }

    // The agent owns the probe; we keep only the command sender.
    let probe = Probe::attach(&chip).unwrap_or_else(|e| {
        eprintln!("attach to {chip}: {e}");
        std::process::exit(1);
    });
    let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();
    let svd_path = env::var("AXYR_SVD").ok();
    // Where the agent may discover flashable firmware images (defaults to the
    // ELF's directory) — so flashing is autonomous, no human supplies a path.
    let firmware_dir = env::var("AXYR_FIRMWARE_DIR")
        .ok()
        .or_else(|| std::path::Path::new(&elf).parent().map(|p| p.display().to_string()));
    let watch = env::var("AXYR_WATCH")
        .map(|s| {
            s.split(',')
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let cfg = Config {
        elf,
        addr2line,
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
    thread::spawn(move || agent::run(probe, cfg, cmd_rx, threads_agent));

    serve_mcp(&crash_file, dts.as_deref(), firmware_dir.as_deref(), &cmd_tx, &threads);
}

/// MCP over stdio: one JSON-RPC message per line.
fn serve_mcp(
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
fn dispatch_tool(
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
