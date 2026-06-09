//! RTOS thread state, parsed from Zephyr's thread-analyzer telemetry.
//!
//! With `CONFIG_THREAD_ANALYZER_AUTO`, the firmware periodically prints one line
//! per thread over RTT, e.g.
//!   ` main                : STACK: unused  688 usage  336 / 1024 ( 32 %); CPU:  25 %`
//! We parse those into per-thread stack usage and CPU load — "what's running,
//! and is any stack about to overflow". Reuses Zephyr; no custom firmware code.

use std::collections::BTreeMap;

/// One thread's resource usage.
#[derive(Clone)]
pub struct ThreadInfo {
    pub name: String,
    pub stack_used: u32,
    pub stack_total: u32,
    pub stack_pct: u32,
    pub cpu_pct: u32,
}

/// The latest snapshot of all known threads (keyed by name, kept up to date as
/// thread-analyzer lines stream in).
#[derive(Default)]
pub struct ThreadTable {
    threads: BTreeMap<String, ThreadInfo>,
}

impl ThreadTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one telemetry line; updates the table if it's a thread-analyzer line.
    pub fn feed(&mut self, line: &str) {
        if let Some(info) = parse_thread_line(line) {
            self.threads.insert(info.name.clone(), info);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.threads.is_empty()
    }

    /// Structured thread list as JSON (for the snapshot contract).
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::Value::Array(
            self.threads
                .values()
                .map(|t| {
                    serde_json::json!({
                        "name": t.name,
                        "stack_used": t.stack_used,
                        "stack_total": t.stack_total,
                        "stack_pct": t.stack_pct,
                        "cpu_pct": t.cpu_pct,
                    })
                })
                .collect(),
        )
    }

    /// Human/agent-readable rendering.
    pub fn render(&self) -> String {
        if self.threads.is_empty() {
            return "No thread data yet.".to_string();
        }
        let mut out = String::from("Threads (stack used/total, CPU load):\n");
        for t in self.threads.values() {
            out.push_str(&format!(
                "  {:<18} stack {}/{} ({}%)  cpu {}%\n",
                t.name, t.stack_used, t.stack_total, t.stack_pct, t.cpu_pct
            ));
        }
        out.trim_end().to_string()
    }
}

/// Parse one thread-analyzer line into a [`ThreadInfo`], or None if it isn't one.
pub fn parse_thread_line(line: &str) -> Option<ThreadInfo> {
    let idx = line.find(": STACK: unused")?;
    let name = line[..idx].trim().to_string();
    if name.is_empty() {
        return None;
    }
    Some(ThreadInfo {
        name,
        stack_used: int_after(line, "usage")?,
        stack_total: int_after(line, "/")?,
        stack_pct: int_after(line, "(")?,
        cpu_pct: int_after(line, "CPU:")?,
    })
}

/// The first integer appearing after `key` in `s`.
fn int_after(s: &str, key: &str) -> Option<u32> {
    let rest = &s[s.find(key)? + key.len()..];
    let digits: String = rest
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &str =
        " main                : STACK: unused  688 usage  336 / 1024 ( 32 %); CPU:  25 %";

    #[test]
    fn parses_a_thread_line() {
        let t = parse_thread_line(LINE).unwrap();
        assert_eq!(t.name, "main");
        assert_eq!(t.stack_used, 336);
        assert_eq!(t.stack_total, 1024);
        assert_eq!(t.stack_pct, 32);
        assert_eq!(t.cpu_pct, 25);
    }

    #[test]
    fn ignores_non_thread_lines() {
        assert!(parse_thread_line("counter=42").is_none());
        assert!(parse_thread_line("Thread analyze:").is_none());
        assert!(parse_thread_line("                     : Total CPU cycles used: 11319").is_none());
    }

    #[test]
    fn table_keeps_latest_per_thread() {
        let mut t = ThreadTable::new();
        t.feed(LINE);
        t.feed(" idle                : STACK: unused  272 usage   48 /  320 ( 15 %); CPU:   0 %");
        let r = t.render();
        assert!(r.contains("main"));
        assert!(r.contains("idle"));
        assert!(r.contains("stack 336/1024"));
    }
}
