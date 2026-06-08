use std::env;
use std::io::{self, Read};
use std::process::Command;
use std::time::Duration;

fn main() {
    // Expect: axyr-engine <serial-port> <elf> <addr2line>
    let args: Vec<String> = env::args().collect();
    if args.len() != 4 {
        eprintln!("usage: {} <serial-port> <elf> <addr2line>", args[0]);
        std::process::exit(1);
    }
    let port_path = args[1].as_str();
    let elf = args[2].as_str();
    let addr2line = args[3].as_str();

    // Open the serial port directly — no more cat/stty.
    let mut port = serialport::new(port_path, 115200)
        .timeout(Duration::from_millis(200))
        .open()
        .expect("failed to open serial port");
    eprintln!("listening on {port_path} @ 115200 ...");

    // Serial is a raw byte stream: read chunks, assemble lines ourselves.
    let mut chunk = [0u8; 256];
    let mut line = String::new();
    loop {
        match port.read(&mut chunk) {
            Ok(0) => {}
            Ok(n) => {
                for &b in &chunk[..n] {
                    match b {
                        b'\n' => {
                            handle_line(line.trim(), elf, addr2line);
                            line.clear();
                        }
                        b'\r' => {} // ignore carriage returns
                        _ => line.push(b as char),
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::TimedOut => {} // no data, keep waiting
            Err(e) => {
                eprintln!("serial error: {e}");
                break;
            }
        }
    }
}

fn handle_line(line: &str, elf: &str, addr2line: &str) {
    let Some(fields) = line.strip_prefix("AXYR_CRASH ") else {
        return; // not a crash line, ignore it
    };

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

    let Some(pc) = pc else {
        eprintln!("crash line has no pc field");
        return;
    };

    let output = Command::new(addr2line)
        .args(["-e", elf, "-f", "-p", pc])
        .output()
        .expect("failed to run addr2line");
    let location = String::from_utf8_lossy(&output.stdout);

    println!("=== AXYR crash report ===");
    println!("reason : {}", reason.unwrap_or("?"));
    println!("pc     : {}", pc);
    println!("where  : {}", location.trim());
}
