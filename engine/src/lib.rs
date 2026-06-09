use std::process::Command;

pub mod coredump;
pub mod recent_log;

/// A crash captured from the device.
pub struct Crash {
    pub reason: Option<u32>,
    pub pc: String,
}

/// Parse one "AXYR_CRASH ..." line into a Crash.
/// Returns None if it isn't a crash line, or if the mandatory `pc` is missing.
pub fn parse_crash_line(line: &str) -> Option<Crash> {
    let fields = line.strip_prefix("AXYR_CRASH ")?;
    let mut pc: Option<&str> = None;
    let mut reason: Option<u32> = None;
    for token in fields.split_whitespace() {
        if let Some((key, value)) = token.split_once('=') {
            match key {
                "pc" => pc = Some(value),
                "reason" => reason = value.parse().ok(),
                _ => {}
            }
        }
    }
    Some(Crash { reason, pc: pc?.to_string() })
}

/// Resolve a program-counter address to "function at file:line" via addr2line.
pub fn symbolize(addr2line: &str, elf: &str, pc: &str) -> String {
    let output = Command::new(addr2line)
        .args(["-e", elf, "-f", "-p", pc])
        .output()
        .expect("failed to run addr2line");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Build the human-readable crash report.
pub fn format_report(crash: &Crash, location: &str) -> String {
    let cause = crash.reason.map(reason_label).unwrap_or("Unknown reason");
    let reason_str = crash
        .reason
        .map(|r| r.to_string())
        .unwrap_or_else(|| "?".to_string());
    format!("Cause: {} [reason {}]\nLocation: {}", cause, reason_str, location)
}

/// Translate a Zephyr fatal-error reason code into a human-readable cause.
/// Generic reasons (0..=4): include/zephyr/fatal_types.h.
/// ARM Cortex-M arch reasons (start at K_ERR_ARCH_START = 16): include/zephyr/arch/arm/arch.h.
pub fn reason_label(reason: u32) -> &'static str {
    match reason {
        // Generic kernel reasons
        0 => "Generic CPU exception",
        1 => "Unhandled (spurious) hardware interrupt",
        2 => "Stack buffer overflow",
        3 => "Kernel oops (software error)",
        4 => "Kernel panic (fatal software error)",

        // ARM Cortex-M — MemManage faults (MPU)
        16 => "Memory protection fault (generic)",
        17 => "Memory protection fault while stacking exception context",
        18 => "Memory protection fault while unstacking exception context",
        19 => "Illegal data access (MPU-protected region)",
        20 => "Illegal instruction fetch (MPU-protected region)",
        21 => "Memory protection fault during lazy FP state save",

        // ARM Cortex-M — Bus faults
        22 => "Bus fault (generic)",
        23 => "Bus fault while stacking exception context",
        24 => "Bus fault while unstacking exception context",
        25 => "Bus fault: invalid memory access (precise)",
        26 => "Bus fault: invalid memory access (imprecise)",
        27 => "Bus fault on instruction fetch",
        28 => "Bus fault during lazy FP state save",

        // ARM Cortex-M — Usage faults
        29 => "Usage fault (generic)",
        30 => "Division by zero",
        31 => "Unaligned memory access",
        32 => "Stack overflow",
        33 => "Coprocessor disabled or absent",
        34 => "Illegal exception return",
        35 => "Illegal EPSR / execution state",
        36 => "Undefined instruction",

        // ARM Cortex-M — TrustZone secure faults
        37..=44 => "TrustZone secure fault",

        // Cortex-A/R (not on Cortex-M, listed for completeness)
        45 => "Undefined instruction",
        46 => "Alignment fault",
        47 => "Background fault",

        _ => "Unknown reason",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_precise_bus_fault() {
        assert_eq!(reason_label(25), "Bus fault: invalid memory access (precise)");
    }

    #[test]
    fn unknown_reason_is_handled() {
        assert_eq!(reason_label(999), "Unknown reason");
    }

    #[test]
    fn parses_a_crash_line() {
        let c = parse_crash_line("AXYR_CRASH v=1 reason=25 pc=0x08000498").unwrap();
        assert_eq!(c.reason, Some(25));
        assert_eq!(c.pc, "0x08000498");
    }

    #[test]
    fn ignores_non_crash_lines() {
        assert!(parse_crash_line("Hello World!").is_none());
    }
}
