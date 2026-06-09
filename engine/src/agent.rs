//! The local agent: the single owner of the debug probe.
//!
//! The SWD link is one shared, inherently serial resource, so exactly one thread
//! owns the [`Probe`] and serializes everything through it. Its loop drains RTT
//! telemetry continuously and, between drains, runs any queued actions
//! (flash / reboot / read). Because RTT is buffered on the target, running an
//! action only pauses the host's draining for the action's duration — no
//! telemetry is lost and the target's real-time behaviour is never disturbed.
//!
//! Callers (e.g. the MCP front-end) never touch the probe: they send an
//! [`Action`] over a channel and await the reply.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;
use std::time::Duration;

use crate::coredump::{CoredumpCollector, CoredumpTools, resolve_backtrace};
use crate::probe::Probe;
use crate::recent_log::RecentLog;
use crate::rtt::Telemetry;
use crate::{format_report, parse_crash_line, symbolize};

/// How many recent telemetry lines to keep as "what was happening just before".
const RECENT_LOG_LINES: usize = 20;

/// An action the agent performs on the board, via the owned probe.
pub enum Action {
    /// Reset the target and let it run.
    Reboot,
    /// Flash an ELF image, then leave the board running.
    Flash(PathBuf),
    /// Read `count` 32-bit words starting at `address`.
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

/// Run the probe-owner loop forever: drain RTT, run queued commands. Consumes
/// the probe (this thread is its sole owner).
pub fn run(mut probe: Probe, cfg: Config, commands: Receiver<Command>) {
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

    let mut collector = CoredumpCollector::new();
    let mut recent_log = RecentLog::new(RECENT_LOG_LINES);
    let mut last_backtrace: Option<String> = None;
    let mut line = String::new();
    let mut buf = [0u8; 1024];

    loop {
        // 1. Drain telemetry (the default, real-time activity).
        if let Ok(n) = telemetry.read(probe.session_mut(), &mut buf) {
            for &b in &buf[..n] {
                match b {
                    b'\n' => {
                        handle_line(line.trim(), &cfg, &mut collector, &mut recent_log, &mut last_backtrace);
                        line.clear();
                    }
                    b'\r' => {}
                    _ => line.push(b as char),
                }
            }
        }

        // 2. Run any queued actions, interleaved between drains. RTT keeps
        //    buffering on the target while we do, so nothing is lost.
        while let Ok(cmd) = commands.try_recv() {
            let result = execute(&mut probe, cmd.action);
            let _ = cmd.reply.send(result);
        }

        thread::sleep(Duration::from_millis(50));
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

/// Process one telemetry line through the crash pipeline (same logic as the old
/// serial listener, now fed from RTT).
fn handle_line(
    line: &str,
    cfg: &Config,
    collector: &mut CoredumpCollector,
    recent_log: &mut RecentLog,
    last_backtrace: &mut Option<String>,
) {
    recent_log.record(line);

    if let (Some(block), Some(tools)) = (collector.feed(line), cfg.coredump.as_ref()) {
        match resolve_backtrace(tools, &block) {
            Ok(bt) => *last_backtrace = Some(bt),
            Err(e) => eprintln!("agent: could not resolve backtrace: {e}"),
        }
    }

    let Some(crash) = parse_crash_line(line) else { return; };
    let location = symbolize(&cfg.addr2line, &cfg.elf, &crash.pc);
    let mut report = format_report(&crash, &location);
    if let Some(bt) = last_backtrace.take() {
        report.push_str("\nCall stack:\n");
        report.push_str(&bt);
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
