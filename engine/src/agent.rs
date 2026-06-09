//! The local agent: the single owner of the debug probe.
//!
//! The SWD link is one shared, inherently serial resource, so exactly one thread
//! owns the [`Probe`] and serializes everything through it. Its loop drains RTT
//! telemetry continuously and, between drains, runs any queued actions
//! (flash / reboot / read). Because RTT is buffered on the target, running an
//! action only pauses the host's draining for the action's duration — no
//! telemetry is lost and the target's real-time behaviour is never disturbed.
//!
//! On a crash, the coredump is NOT streamed over RTT (slow): the firmware's
//! IN_MEMORY backend leaves it in a RAM buffer, and the agent reads that buffer
//! in one SWD block (the core is halted after the fault) — fast and log-free.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::coredump::{CoredumpTools, resolve_backtrace_from_bin};
use crate::probe::Probe;
use crate::recent_log::RecentLog;
use crate::rtt::Telemetry;
use crate::threads::ThreadTable;
use crate::{Crash, format_report, parse_crash_line, symbolize, symbols};

/// How many recent telemetry lines to keep as "what was happening just before".
const RECENT_LOG_LINES: usize = 20;
/// Zephyr's IN_MEMORY coredump backend buffer symbol.
const COREDUMP_SYMBOL: &str = "in_memory_coredump";

/// An action the agent performs on the board, via the owned probe.
pub enum Action {
    Reboot,
    Flash(PathBuf),
    ReadMemory { address: u64, count: usize },
}

/// A request to the agent: an action plus a channel to send the result back.
pub struct Command {
    pub action: Action,
    pub reply: Sender<Result<String, String>>,
}

/// Static configuration the owner loop needs for the crash pipeline.
pub struct Config {
    pub elf: String,
    pub addr2line: String,
    pub crash_file: String,
    pub coredump: Option<CoredumpTools>,
}

/// Run the probe-owner loop forever: drain RTT, capture crashes, run commands.
/// `threads` is shared with the MCP front-end so `get_threads` can read it.
pub fn run(
    mut probe: Probe,
    cfg: Config,
    commands: Receiver<Command>,
    threads: Arc<Mutex<ThreadTable>>,
) {
    // Where the IN_MEMORY coredump buffer lives in RAM (resolved from the ELF).
    let coredump_addr = symbols::resolve(&cfg.elf, COREDUMP_SYMBOL)
        .map(|s| s.address)
        .map_err(|e| eprintln!("agent: no coredump buffer symbol ({e}); call stack disabled"))
        .ok();

    // The RTT scan needs an awake window; retry until the control block is found.
    let mut telemetry = loop {
        match Telemetry::attach(probe.session_mut()) {
            Ok(t) => break t,
            Err(e) => {
                eprintln!("agent: attach RTT: {e}; retrying...");
                thread::sleep(Duration::from_millis(300));
            }
        }
    };
    eprintln!("agent: RTT attached; streaming telemetry (no halt)");

    let mut recent_log = RecentLog::new(RECENT_LOG_LINES);
    let mut line = String::new();
    let mut buf = [0u8; 1024];

    loop {
        // 1. Drain telemetry fully this cycle. A crash is signalled by the
        //    AXYR_CRASH line; the heavy work happens after the drain.
        let mut crash: Option<Crash> = None;
        loop {
            match telemetry.read(probe.session_mut(), &mut buf) {
                Ok(n) if n > 0 => {
                    for &b in &buf[..n] {
                        match b {
                            b'\n' => {
                                if let Some(c) = process_line(line.trim(), &mut recent_log, &threads) {
                                    crash = Some(c);
                                }
                                line.clear();
                            }
                            b'\r' => {}
                            _ => line.push(b as char),
                        }
                    }
                }
                _ => break,
            }
        }

        // 2. On a crash, read the coredump from RAM over SWD and write the report.
        if let Some(crash) = crash {
            report_crash(&mut probe, &cfg, coredump_addr, &crash, &recent_log);
        }

        // 3. Run any queued actions, interleaved between drains. RTT keeps
        //    buffering on the target while we do, so nothing is lost.
        while let Ok(cmd) = commands.try_recv() {
            let result = execute(&mut probe, cmd.action);
            let _ = cmd.reply.send(result);
        }

        thread::sleep(Duration::from_millis(10));
    }
}

/// Record a telemetry line (recent log + thread state) and, if it is the crash
/// packet, return the parsed crash.
fn process_line(
    line: &str,
    recent_log: &mut RecentLog,
    threads: &Arc<Mutex<ThreadTable>>,
) -> Option<Crash> {
    recent_log.record(line);
    if let Ok(mut t) = threads.lock() {
        t.feed(line);
    }
    parse_crash_line(line)
}

/// Build and write the crash report: decode cause + location, read the coredump
/// from RAM over SWD and unwind it, and attach the recent telemetry.
fn report_crash(
    probe: &mut Probe,
    cfg: &Config,
    coredump_addr: Option<u64>,
    crash: &Crash,
    recent_log: &RecentLog,
) {
    let location = symbolize(&cfg.addr2line, &cfg.elf, &crash.pc);
    let mut report = format_report(crash, &location);

    // Read the in-memory coredump (core is halted after the fault) and unwind it.
    if let (Some(addr), Some(tools)) = (coredump_addr, cfg.coredump.as_ref()) {
        match probe.read_in_memory_coredump(addr) {
            Ok(Some(dump)) => match resolve_backtrace_from_bin(tools, &dump) {
                Ok(bt) => {
                    report.push_str("\nCall stack:\n");
                    report.push_str(&bt);
                }
                Err(e) => eprintln!("agent: backtrace: {e}"),
            },
            Ok(None) => eprintln!("agent: no valid coredump in RAM"),
            Err(e) => eprintln!("agent: read coredump: {e}"),
        }
    }

    if !recent_log.is_empty() {
        report.push_str("\nRecent telemetry:\n");
        report.push_str(&recent_log.snapshot());
    }

    println!("=== AXYR crash report ===\n{report}");
    if let Err(e) = std::fs::write(&cfg.crash_file, report.as_bytes()) {
        eprintln!("agent: could not write crash file: {e}");
    }
}

/// Execute one action on the owned probe.
fn execute(probe: &mut Probe, action: Action) -> Result<String, String> {
    match action {
        Action::Reboot => {
            probe.reset()?;
            Ok("Board reset and running.".to_string())
        }
        Action::Flash(path) => {
            probe.flash_elf(&path)?;
            Ok(format!("Flashed {}; board running.", path.display()))
        }
        Action::ReadMemory { address, count } => {
            let mut words = vec![0u32; count];
            probe.read_words(address, &mut words)?;
            let mut out = String::new();
            for (i, w) in words.iter().enumerate() {
                out.push_str(&format!("{:#010x}: {w:#010x}\n", address + (i as u64) * 4));
            }
            Ok(out.trim_end().to_string())
        }
    }
}
