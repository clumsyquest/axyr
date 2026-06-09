use std::env;
use std::fs;
use std::io::{self, BufRead, Write};

use serde_json::{json, Value};

fn main() {
    // The crash file that the serial listener keeps up to date.
    let crash_file = env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: mcp <crash-file>");
        std::process::exit(1);
    });

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
                "result": { "tools": [{
                    "name": "get_last_crash",
                    "description": "Return the most recent crash captured from the board (fault type and source location).",
                    "inputSchema": { "type": "object", "properties": {} }
                }]}
            })),
            "tools/call" => {
                let tool = req.pointer("/params/name").and_then(Value::as_str).unwrap_or("");
                let text = if tool == "get_last_crash" {
                    fs::read_to_string(&crash_file)
                        .unwrap_or_else(|_| "No crash recorded yet.".to_string())
                } else {
                    format!("Unknown tool: {tool}")
                };
                Some(json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": { "content": [{ "type": "text", "text": text }] }
                }))
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
