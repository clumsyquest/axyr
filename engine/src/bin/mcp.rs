//! MCP server exposing Axyr to an AI agent over stdio (one JSON-RPC message per
//! line).
//!
//! Read tools:
//!   - get_last_crash : the most recent crash post-mortem.
//!
//! Action tools (drive the board over the debug probe):
//!   - reboot_board   : reset the target and let it run.
//!   - flash_firmware : flash an ELF onto the target.
//!   - read_memory    : read 32-bit words from target memory (live).
//!
//! Each action attaches the probe for the call and releases it on return, so it
//! never holds the SWD lock between calls (the serial listener uses a different
//! USB interface and keeps running in parallel).

use std::env;
use std::fs;
use std::io::{self, BufRead, Write};

use serde_json::{Value, json};

use axyr_engine::probe::{DEFAULT_CHIP, Probe};
use axyr_engine::{symbols, system_map};

fn main() {
    // The crash file that the serial listener keeps up to date.
    let crash_file = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: mcp <crash-file>");
        std::process::exit(1);
    });
    // The target chip for probe actions (overridable for other boards).
    let chip = env::var("AXYR_CHIP").unwrap_or_else(|_| DEFAULT_CHIP.to_string());
    // Path to the Zephyr devicetree (build/zephyr/zephyr.dts) for the system map.
    let dts = env::var("AXYR_DTS").ok();
    // Path to the firmware ELF, for resolving variables to read live.
    let elf = env::var("AXYR_ELF").ok();

    let stdin = io::stdin();
    let stdout = io::stdout();

    // MCP over stdio = one JSON-RPC message per line.
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
        let id = req.get("id").cloned(); // absent for notifications

        let response: Option<Value> = match method {
            "initialize" => {
                // Echo the client's protocol version back, so it always matches.
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
                "jsonrpc": "2.0", "id": id,
                "result": { "tools": tool_definitions() }
            })),
            "tools/call" => {
                let tool = req.pointer("/params/name").and_then(Value::as_str).unwrap_or("");
                let args = req.pointer("/params/arguments").cloned().unwrap_or(json!({}));
                let result =
                    dispatch_tool(tool, &args, &crash_file, &chip, dts.as_deref(), elf.as_deref());
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
            // notifications/initialized and anything else: no reply.
            _ => None,
        };

        if let Some(resp) = response {
            let mut out = stdout.lock();
            writeln!(out, "{resp}").expect("write failed");
            out.flush().expect("flush failed");
        }
    }
}

/// The tool catalogue advertised to the agent.
fn tool_definitions() -> Value {
    json!([
        {
            "name": "get_last_crash",
            "description": "Return the most recent crash captured from the board (cause, source location, call stack, and recent serial output).",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "get_system_map",
            "description": "Return the board's hardware map from the Zephyr devicetree: the peripherals (I2C/SPI/UART/timers/GPIO) and the sensors/actuators on them, with addresses and on/off state.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "reboot_board",
            "description": "Reset the target board over the debug probe and let it run.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "flash_firmware",
            "description": "Flash an ELF image onto the target board's flash, then leave it running.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute path to the ELF file to flash." }
                },
                "required": ["path"]
            }
        },
        {
            "name": "read_memory",
            "description": "Read 32-bit words from the target's memory over SWD (works while the core runs). Useful to inspect registers, peripherals, or variables.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "address": { "type": "string", "description": "Start address, e.g. \"0xE000ED00\" or decimal." },
                    "count": { "type": "integer", "description": "Number of 32-bit words to read (default 1, max 256)." }
                },
                "required": ["address"]
            }
        },
        {
            "name": "read_variable",
            "description": "Read a firmware global variable live by name: resolves its address from the ELF symbol table, then reads it over SWD while the core runs.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "The global variable's symbol name, e.g. \"axyr_counter\"." }
                },
                "required": ["name"]
            }
        }
    ])
}

/// Route a tool call to its handler. Returns Ok(text) or Err(text); the caller
/// maps Err to an MCP error result so the agent sees the failure as data.
fn dispatch_tool(
    tool: &str,
    args: &Value,
    crash_file: &str,
    chip: &str,
    dts: Option<&str>,
    elf: Option<&str>,
) -> Result<String, String> {
    match tool {
        "get_last_crash" => Ok(fs::read_to_string(crash_file)
            .unwrap_or_else(|_| "No crash recorded yet.".to_string())),
        "get_system_map" => {
            let path = dts.ok_or("system map not configured (set AXYR_DTS)")?;
            let dts_src = fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
            system_map::render(&dts_src).ok_or_else(|| "could not parse devicetree".to_string())
        }
        "reboot_board" => {
            let mut probe = Probe::attach(chip)?;
            probe.reset()?;
            Ok("Board reset and running.".to_string())
        }
        "flash_firmware" => {
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .ok_or("missing required argument: path")?;
            let mut probe = Probe::attach(chip)?;
            probe.flash_elf(std::path::Path::new(path))?;
            Ok(format!("Flashed {path} to {chip}; board running."))
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
            let mut probe = Probe::attach(chip)?;
            let mut words = vec![0u32; count];
            probe.read_words(address, &mut words)?;
            let mut out = String::new();
            for (i, w) in words.iter().enumerate() {
                let a = address + (i as u64) * 4;
                out.push_str(&format!("{a:#010x}: {w:#010x}\n"));
            }
            Ok(out.trim_end().to_string())
        }
        "read_variable" => {
            let elf = elf.ok_or("variable read not configured (set AXYR_ELF)")?;
            let name = args
                .get("name")
                .and_then(Value::as_str)
                .ok_or("missing required argument: name")?;
            let sym = symbols::resolve(elf, name)?;
            // Read enough 32-bit words to cover the variable (min 1, capped).
            let words_needed = (sym.size.max(1) as usize).div_ceil(4).clamp(1, 64);
            let mut probe = Probe::attach(chip)?;
            let mut words = vec![0u32; words_needed];
            // Snapshot read: SRAM is only debugger-visible while halted, so this
            // briefly halts, samples, and resumes.
            probe.read_words_snapshot(sym.address, &mut words)?;
            let mut out = format!(
                "{} @{:#010x} ({} bytes) =",
                sym.name, sym.address, sym.size
            );
            if sym.size <= 4 {
                out.push_str(&format!(" {:#010x} ({})", words[0], words[0]));
            } else {
                for w in &words {
                    out.push_str(&format!(" {w:#010x}"));
                }
            }
            Ok(out)
        }
        other => Err(format!("Unknown tool: {other}")),
    }
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
