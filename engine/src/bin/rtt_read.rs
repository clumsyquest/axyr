//! Smoke test for the RTT telemetry reader: stream the firmware's RTT output
//! to stdout, non-intrusively (the core is never halted).
//!
//!   rtt_read [chip]   (auto-detects the target when omitted)

use std::io::{self, Write};
use std::{thread, time::Duration};

use axyr_engine::probe::Probe;
use axyr_engine::rtt::Telemetry;

fn main() {
    let mut probe = match std::env::args().nth(1) {
        Some(chip) => Probe::attach(&chip),
        None => Probe::attach_auto(None).map(|(p, _)| p),
    }
    .unwrap_or_else(|e| {
        eprintln!("attach: {e}");
        std::process::exit(1);
    });

    // The RTT scan needs an awake window; retry until the control block is found.
    let mut telemetry = loop {
        match Telemetry::attach(probe.session_mut()) {
            Ok(t) => break t,
            Err(e) => {
                eprintln!("{e}; retrying...");
                thread::sleep(Duration::from_millis(300));
            }
        }
    };
    eprintln!("RTT attached; streaming telemetry (no halt)");

    let mut buf = [0u8; 1024];
    loop {
        match telemetry.read(probe.session_mut(), &mut buf) {
            Ok(n) if n > 0 => {
                io::stdout().write_all(&buf[..n]).ok();
                io::stdout().flush().ok();
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("read: {e}");
                break;
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
}
