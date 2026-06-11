pub mod agent;
pub mod api;
pub mod chip;
pub mod coredump;
pub mod link;
pub mod relay;
pub mod remote;
pub mod wire;
pub mod diff;
pub mod health;
pub mod peripheral;
pub mod probe;
pub mod recent_log;
pub mod rtt;
pub mod serial;
pub mod symbols;
pub mod threads;
pub mod system_map;
pub mod trace;
pub mod unwind;

/// A crash captured from the device.
#[derive(Clone)]
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

/// Resolve a program-counter address to "function at file:line", in-process via
/// the `addr2line` crate (DWARF) — no external `addr2line` binary needed.
pub fn symbolize(elf: &str, pc: &str) -> String {
    let addr = u64::from_str_radix(pc.trim().trim_start_matches("0x").trim_start_matches("0X"), 16)
        .unwrap_or(0);
    let loader = match addr2line::Loader::new(elf) {
        Ok(l) => l,
        Err(e) => return format!("?? (symbolize {elf}: {e})"),
    };
    let func = loader.find_symbol(addr).unwrap_or("??").to_string();
    match loader.find_location(addr) {
        Ok(Some(loc)) => format!("{func} at {}:{}", loc.file.unwrap_or("?"), loc.line.unwrap_or(0)),
        _ => format!("{func} at ?"),
    }
}

/// Build the human-readable crash report. `arm` says whether the firmware
/// targets ARM (see [`reason_label`]).
pub fn format_report(crash: &Crash, location: &str, arm: bool) -> String {
    let cause = crash.reason.map(|r| reason_label(r, arm)).unwrap_or("Unknown reason");
    let reason_str = crash
        .reason
        .map(|r| r.to_string())
        .unwrap_or_else(|| "?".to_string());
    format!("Cause: {} [reason {}]\nLocation: {}", cause, reason_str, location)
}

/// Translate a Zephyr fatal-error reason code into a human-readable cause.
/// Generic reasons (0..=4): include/zephyr/fatal_types.h — valid on every
/// architecture. Codes from K_ERR_ARCH_START (16) up are defined PER
/// architecture; the tables below are ARM's (include/zephyr/arch/arm/arch.h),
/// so they only apply when `arm` is true — an honest "not decoded" beats a
/// wrong ARM fault name on another architecture.
pub fn reason_label(reason: u32, arm: bool) -> &'static str {
    match reason {
        // Generic kernel reasons
        0 => "Generic CPU exception",
        1 => "Unhandled (spurious) hardware interrupt",
        2 => "Stack buffer overflow",
        3 => "Kernel oops (software error)",
        4 => "Kernel panic (fatal software error)",

        _ if !arm => "Architecture-specific fault (not decoded for this architecture yet)",

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
        assert_eq!(reason_label(25, true), "Bus fault: invalid memory access (precise)");
    }

    #[test]
    fn unknown_reason_is_handled() {
        assert_eq!(reason_label(999, true), "Unknown reason");
    }

    #[test]
    fn never_applies_arm_fault_names_off_arm() {
        // Generic kernel codes still decode; ARM-specific ones must not.
        assert_eq!(reason_label(2, false), "Stack buffer overflow");
        assert_eq!(
            reason_label(25, false),
            "Architecture-specific fault (not decoded for this architecture yet)"
        );
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
