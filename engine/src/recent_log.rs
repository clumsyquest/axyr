//! Recent serial output ("what was happening just before the crash").
//!
//! The engine already sees every serial line, so it keeps a bounded ring buffer
//! of the most recent ones and attaches them to the crash report. This captures
//! the program's narrative leading up to the fault — for both the human and the
//! AI agent — without any extra firmware support.

use std::collections::VecDeque;

use crate::coredump::strip_ansi;

/// A bounded ring buffer of the most recent serial lines.
pub struct RecentLog {
    capacity: usize,
    lines: VecDeque<String>,
}

impl RecentLog {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            lines: VecDeque::with_capacity(capacity.max(1)),
        }
    }

    /// Record a serial line, dropping the oldest if at capacity.
    ///
    /// Crash *machinery* is skipped: the coredump block (`#CD:`) is binary
    /// noise, and the `AXYR_CRASH` packet is our own marker — neither is "log"
    /// output the firmware meant to show. ANSI color codes are stripped so the
    /// captured log reads cleanly.
    pub fn record(&mut self, line: &str) {
        let line = strip_ansi(line);
        let line = line.trim();
        if line.is_empty() || line.starts_with("AXYR_CRASH") || line.contains("#CD:") {
            return;
        }
        if self.lines.len() == self.capacity {
            self.lines.pop_front();
        }
        self.lines.push_back(line.to_string());
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// The buffered lines, oldest first, newline-joined.
    pub fn snapshot(&self) -> String {
        self.lines.iter().cloned().collect::<Vec<_>>().join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_the_last_n_lines() {
        let mut log = RecentLog::new(2);
        log.record("one");
        log.record("two");
        log.record("three");
        assert_eq!(log.snapshot(), "two\nthree");
    }

    #[test]
    fn skips_crash_machinery_and_blanks() {
        let mut log = RecentLog::new(10);
        log.record("Booting Zephyr OS");
        log.record("About to crash...");
        log.record(""); // blank
        log.record("[00:00:00] <err> coredump: #CD:5a4502"); // coredump noise
        log.record("AXYR_CRASH v=1 reason=25 pc=0x08000498"); // our packet
        assert_eq!(log.snapshot(), "Booting Zephyr OS\nAbout to crash...");
    }

    #[test]
    fn strips_ansi_color_codes() {
        let mut log = RecentLog::new(10);
        log.record("\x1b[1;31mAbout to crash...\x1b[0m");
        assert_eq!(log.snapshot(), "About to crash...");
    }

    #[test]
    fn empty_until_something_is_recorded() {
        let mut log = RecentLog::new(5);
        assert!(log.is_empty());
        log.record("hello");
        assert!(!log.is_empty());
    }
}
