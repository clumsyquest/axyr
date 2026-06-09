use std::env;
use std::fs;
use std::io::{self, Read};
use std::process::Command;
use std::time::Duration;

fn main() {
    // Expect: axyr-engine <serial-port> <elf> <addr2line> <crash-file>
    let args: Vec<String> = env::args().collect();
    if args.len() != 5 {
        eprintln!("usage: {} <serial-port> <elf> <addr2line> <crash-file>", args[0]);
        std::process::exit(1);
    }
    let port_path = args[1].as_str();
    let elf = args[2].as_str();
    let addr2line = args[3].as_str();
    let crash_file = args[4].as_str();

    let mut port = serialport::new(port_path, 115200)
        .timeout(Duration::from_millis(200))
        .open()
        .expect("failed to open serial port");
    eprintln!("listening on {port_path} @ 115200 ...");

    let mut chunk = [0u8; 256];
    let mut line = String::new();
    loop {
        match port.read(&mut chunk) {
            Ok(0) => {}
            Ok(n) => {
                for &b in &chunk[..n] {
                    match b {
                        b'\n' => {
                            handle_line(line.trim(), elf, addr2line, crash_file);
                            line.clear();
                        }
                        b'\r' => {}
                        _ => line.push(b as char),
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::TimedOut => {}
            Err(e) => { eprintln!("serial error: {e}"); break; }
        }
    }
}

fn handle_line(line: &str, elf: &str, addr2line: &str, crash_file: &str) {
    let Some(fields) = line.strip_prefix("AXYR_CRASH ") else { return; };

    let mut pc: Option<&str> = None;
    let mut reason: Option<&str> = None;
    for token in fields.split_whitespace() {
        if let Some((key, value)) = token.split_once('=') {
            match key {
                "pc" => pc = Some(value),
                "reason" => reason = Some(value),
                _ => {}
            }
        }
    }

    let Some(pc) = pc else { eprintln!("crash line has no pc field"); return; };

    let output = Command::new(addr2line)
        .args(["-e", elf, "-f", "-p", pc])
        .output()
        .expect("failed to run addr2line");
    let location = String::from_utf8_lossy(&output.stdout);
    let location = location.trim();

    // Build a one-line report: print it AND save it as "the last crash".
    let report = format!("reason {} — {}", reason.unwrap_or("?"), location);
    println!("=== AXYR crash report ===\n{report}");
    if let Err(e) = fs::write(crash_file, report.as_bytes()) {
        eprintln!("could not write crash file: {e}");
    }
}
