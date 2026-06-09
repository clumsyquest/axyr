use std::env;
use std::fs;
use std::io::{self, Read};
use std::time::Duration;

use axyr_engine::coredump::{CoredumpCollector, CoredumpTools, resolve_backtrace};
use axyr_engine::recent_log::RecentLog;
use axyr_engine::{format_report, parse_crash_line, symbolize};

/// How many recent serial lines to keep as "what was happening just before".
const RECENT_LOG_LINES: usize = 20;

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

    // Optional: coredump tooling for full call-stack resolution. If it isn't
    // configured (see CoredumpTools::from_env), we still produce the fast,
    // location-only report.
    let cd_tools = CoredumpTools::from_env(elf);
    if cd_tools.is_none() {
        eprintln!("note: coredump tools not configured; call stack disabled");
    }
    let mut collector = CoredumpCollector::new();
    // Holds the most recent backtrace; the coredump block arrives just before
    // the AXYR_CRASH line, so it is ready by the time we build the report.
    let mut last_backtrace: Option<String> = None;
    // Rolling window of recent serial output, attached to the next crash report.
    let mut recent_log = RecentLog::new(RECENT_LOG_LINES);

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
                            handle_line(
                                line.trim(),
                                elf,
                                addr2line,
                                crash_file,
                                cd_tools.as_ref(),
                                &mut collector,
                                &mut last_backtrace,
                                &mut recent_log,
                            );
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

#[allow(clippy::too_many_arguments)]
fn handle_line(
    line: &str,
    elf: &str,
    addr2line: &str,
    crash_file: &str,
    cd_tools: Option<&CoredumpTools>,
    collector: &mut CoredumpCollector,
    last_backtrace: &mut Option<String>,
    recent_log: &mut RecentLog,
) {
    // Keep the rolling window of serial output up to date (the recorder skips
    // crash machinery itself).
    recent_log.record(line);

    // Feed the coredump collector first; a full block resolves into a backtrace
    // that we attach to the next crash report.
    if let (Some(block), Some(tools)) = (collector.feed(line), cd_tools) {
        match resolve_backtrace(tools, &block) {
            Ok(bt) => *last_backtrace = Some(bt),
            Err(e) => eprintln!("coredump: could not resolve backtrace: {e}"),
        }
    }

    let Some(crash) = parse_crash_line(line) else { return; };
    let location = symbolize(addr2line, elf, &crash.pc);
    let mut report = format_report(&crash, &location);
    if let Some(bt) = last_backtrace.take() {
        report.push_str("\nCall stack:\n");
        report.push_str(&bt);
    }
    if !recent_log.is_empty() {
        report.push_str("\nRecent serial output:\n");
        report.push_str(&recent_log.snapshot());
    }

    println!("=== AXYR crash report ===\n{report}");
    if let Err(e) = fs::write(crash_file, report.as_bytes()) {
        eprintln!("could not write crash file: {e}");
    }
}
