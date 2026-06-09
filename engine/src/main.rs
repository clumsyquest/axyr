use std::env;
use std::fs;
use std::io::{self, Read};
use std::time::Duration;

use axyr_engine::{format_report, parse_crash_line, symbolize};

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
    let Some(crash) = parse_crash_line(line) else { return; };
    let location = symbolize(addr2line, elf, &crash.pc);
    let report = format_report(&crash, &location);
    println!("=== AXYR crash report ===\n{report}");
    if let Err(e) = fs::write(crash_file, report.as_bytes()) {
        eprintln!("could not write crash file: {e}");
    }
}
