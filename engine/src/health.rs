//! Proactive health checks — turn raw state into insight.
//!
//! Axyr doesn't just expose data; it flags problems a developer would have to
//! hunt for: a stack about to overflow, a runaway/starved thread, a watchdog or
//! brown-out reset, an active crash. The checks run on data we already capture.

use crate::threads::ThreadInfo;

#[derive(PartialEq, Eq, Debug)]
pub enum Severity {
    Critical,
    Warning,
    Info,
}

impl Severity {
    fn tag(&self) -> &'static str {
        match self {
            Severity::Critical => "CRITICAL",
            Severity::Warning => "WARNING",
            Severity::Info => "info",
        }
    }
}

pub struct Finding {
    pub severity: Severity,
    pub message: String,
}

/// Run all checks over the captured state and return findings, worst first.
pub fn analyze(
    threads: &[ThreadInfo],
    reset_reason: Option<&str>,
    crash_cause: Option<&str>,
) -> Vec<Finding> {
    let mut f = Vec::new();

    if let Some(cause) = crash_cause {
        f.push(Finding {
            severity: Severity::Critical,
            message: format!("Board has crashed: {cause}"),
        });
    }

    for t in threads {
        if t.stack_pct >= 90 {
            let sev = if t.stack_pct >= 95 { Severity::Critical } else { Severity::Warning };
            f.push(Finding {
                severity: sev,
                message: format!(
                    "Thread '{}' stack {}% used ({}/{} bytes) — near overflow",
                    t.name, t.stack_pct, t.stack_used, t.stack_total
                ),
            });
        }
        if t.name != "idle" && t.cpu_pct >= 95 {
            f.push(Finding {
                severity: Severity::Warning,
                message: format!(
                    "Thread '{}' at {}% CPU — possible runaway or starving others",
                    t.name, t.cpu_pct
                ),
            });
        }
    }

    if let Some(r) = reset_reason {
        if r.contains("IWDG") || r.contains("WWDG") {
            f.push(Finding {
                severity: Severity::Warning,
                message: format!("Last reset was a watchdog reset ({r})"),
            });
        }
        if r.contains("BOR") {
            f.push(Finding {
                severity: Severity::Warning,
                message: format!("Last reset was a brown-out (power) reset ({r})"),
            });
        }
    }

    // Worst first.
    f.sort_by_key(|x| match x.severity {
        Severity::Critical => 0,
        Severity::Warning => 1,
        Severity::Info => 2,
    });
    f
}

/// Render findings for the human / agent.
pub fn render(findings: &[Finding]) -> String {
    if findings.is_empty() {
        return "Health: OK — no anomalies detected.".to_string();
    }
    let mut out = String::from("Health findings:\n");
    for f in findings {
        out.push_str(&format!("  [{}] {}\n", f.severity.tag(), f.message));
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread(name: &str, stack_pct: u32, cpu_pct: u32) -> ThreadInfo {
        ThreadInfo {
            name: name.to_string(),
            stack_used: stack_pct * 10,
            stack_total: 1000,
            stack_pct,
            cpu_pct,
        }
    }

    #[test]
    fn flags_stack_near_overflow() {
        let f = analyze(&[thread("main", 96, 10)], None, None);
        assert_eq!(f[0].severity, Severity::Critical);
        assert!(f[0].message.contains("near overflow"));
    }

    #[test]
    fn flags_watchdog_reset_and_crash_order() {
        let f = analyze(&[thread("main", 10, 10)], Some("IWDGRSTF"), Some("Bus fault"));
        // Crash (critical) sorts before the watchdog warning.
        assert_eq!(f[0].severity, Severity::Critical);
        assert!(f[0].message.contains("crashed"));
        assert!(f.iter().any(|x| x.message.contains("watchdog")));
    }

    #[test]
    fn healthy_system_has_no_findings() {
        let f = analyze(&[thread("main", 30, 5), thread("idle", 10, 95)], Some("PORRSTF"), None);
        assert!(f.is_empty());
        assert!(render(&f).contains("OK"));
    }
}
