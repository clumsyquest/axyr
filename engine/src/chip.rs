//! Chip identification — turning "whatever board is plugged in" into a precise
//! probe-rs target, without the user naming it.
//!
//! Two independent signals, used in this order by [`crate::probe::Probe::attach_auto`]:
//!
//! 1. **The firmware build.** Zephyr's `build/zephyr/.config` records the exact
//!    SoC the user compiled for (`CONFIG_SOC="stm32f401xe"`). That string is
//!    already in probe-rs's chip-name convention (lowercase `x` = wildcard), so
//!    it resolves directly against the target registry. No silicon access needed.
//! 2. **The silicon itself.** probe-rs's `TargetSelector::Auto` covers vendors
//!    with detection sequences (Nordic, Espressif, Infineon, Microchip…), but
//!    not ST. For ST we attach a generic Cortex-M target, read the DBGMCU
//!    IDCODE (its low 12 bits are the device id, unique across the STM32 line),
//!    and map it to a registry target here.
//!
//! Both signals can disagree (firmware built for chip X, board is chip Y) —
//! [`st_devid_matches`] lets the caller surface that instead of flashing the
//! wrong die.

use probe_rs::config::Registry;

/// DBGMCU IDCODE addresses across STM32 families. The device id lives in the
/// low 12 bits at one of these, depending on the family:
/// `0xE004_2000` (F1/F2/F3/F4/F7/L1/L4/G4/WB/WL), `0x4001_5800` (Cortex-M0/M0+
/// parts: F0/L0/G0/C0), `0x5C00_1000` (H7), `0xE004_4000` (Armv8-M parts:
/// L5/H5/U5). Reads at the wrong one fail or return an unknown id — both are
/// treated as "keep looking".
pub const ST_DBGMCU_ADDRS: &[u64] = &[0xE004_2000, 0x4001_5800, 0x5C00_1000, 0xE004_4000];

/// Generic ARM targets to try (in order) when the chip is unknown: enough to
/// read memory on any Cortex-M, which is all detection and observation need.
pub const CORTEX_GENERICS: &[&str] =
    &["Cortex-M4", "Cortex-M7", "Cortex-M33", "Cortex-M3", "Cortex-M0+", "Cortex-M0"];

/// STM32 device id (DBGMCU IDCODE low 12 bits) → registry search queries, most
/// representative part first. Lowercase `x` is a registry wildcard, so e.g.
/// `STM32F401xE` names "F401, any package, 512K" exactly like ST's datasheets.
/// Ids cross-checked against ST reference manuals (and OpenOCD/pyOCD's tables).
/// One id can cover several parts (0x413 = F405/407/415/417): they share the
/// memory map and flash algorithm, so a representative is safe to attach.
fn st_candidates(devid: u32) -> &'static [&'static str] {
    match devid & 0xfff {
        // F0
        0x440 => &["STM32F051", "STM32F058"],
        0x442 => &["STM32F091", "STM32F030xC"],
        0x444 => &["STM32F031", "STM32F030x6"],
        0x445 => &["STM32F042", "STM32F070x6"],
        0x448 => &["STM32F072", "STM32F071", "STM32F070xB"],
        // F1
        0x410 => &["STM32F103xB"],
        0x412 => &["STM32F103x6"],
        0x414 => &["STM32F103xE"],
        0x418 => &["STM32F107", "STM32F105"],
        0x420 => &["STM32F100x8"],
        0x428 => &["STM32F100xE"],
        0x430 => &["STM32F103xG"],
        // F2
        0x411 => &["STM32F207", "STM32F205"],
        // F3
        0x422 => &["STM32F303xC", "STM32F302xC"],
        0x432 => &["STM32F373"],
        0x438 => &["STM32F334", "STM32F303x8"],
        0x439 => &["STM32F302x8", "STM32F301"],
        0x446 => &["STM32F303xE", "STM32F302xE"],
        // F4
        0x413 => &["STM32F407", "STM32F405", "STM32F417"],
        0x419 => &["STM32F429", "STM32F427", "STM32F439"],
        0x421 => &["STM32F446"],
        0x423 => &["STM32F401xC"],
        0x431 => &["STM32F411"],
        0x433 => &["STM32F401xE"],
        0x434 => &["STM32F469", "STM32F479"],
        0x441 => &["STM32F412"],
        0x458 => &["STM32F410"],
        0x463 => &["STM32F413", "STM32F423"],
        // F7
        0x449 => &["STM32F746", "STM32F745", "STM32F756"],
        0x451 => &["STM32F767", "STM32F769", "STM32F777"],
        0x452 => &["STM32F722", "STM32F723", "STM32F730"],
        // G0
        0x456 => &["STM32G051", "STM32G061"],
        0x460 => &["STM32G071", "STM32G081"],
        0x466 => &["STM32G031", "STM32G041"],
        0x467 => &["STM32G0B1", "STM32G0C1"],
        // G4
        0x468 => &["STM32G431", "STM32G441"],
        0x469 => &["STM32G474", "STM32G473", "STM32G484"],
        0x479 => &["STM32G491", "STM32G4A1"],
        // H5
        0x474 => &["STM32H503"],
        0x484 => &["STM32H563", "STM32H573", "STM32H562"],
        // H7
        0x450 => &["STM32H743", "STM32H745", "STM32H750", "STM32H753"],
        0x480 => &["STM32H7A3", "STM32H7B3", "STM32H7B0"],
        0x483 => &["STM32H723", "STM32H725", "STM32H730", "STM32H735"],
        // L0
        0x417 => &["STM32L053", "STM32L063"],
        0x425 => &["STM32L031", "STM32L041"],
        0x447 => &["STM32L071", "STM32L072", "STM32L081"],
        0x457 => &["STM32L011", "STM32L021"],
        // L1
        0x416 => &["STM32L151xB"],
        0x429 => &["STM32L151xB"],
        0x427 => &["STM32L151xC"],
        0x436 => &["STM32L152xD", "STM32L151xD"],
        0x437 => &["STM32L151xE", "STM32L152xE"],
        // L4 / L4+
        0x415 => &["STM32L476", "STM32L475", "STM32L486"],
        0x461 => &["STM32L496", "STM32L4A6"],
        0x462 => &["STM32L452", "STM32L462", "STM32L451"],
        0x464 => &["STM32L412", "STM32L422"],
        0x470 => &["STM32L4R5", "STM32L4S5"],
        0x471 => &["STM32L4P5", "STM32L4Q5"],
        // L5
        0x472 => &["STM32L552", "STM32L562"],
        // C0
        0x443 => &["STM32C011"],
        0x453 => &["STM32C031"],
        // U5
        0x482 => &["STM32U575", "STM32U585"],
        0x481 => &["STM32U595", "STM32U5A5", "STM32U599"],
        // WB / WL
        0x494 => &["STM32WB15", "STM32WB10"],
        0x495 => &["STM32WB55", "STM32WB35"],
        0x497 => &["STM32WLE5", "STM32WL55"],
        _ => &[],
    }
}

/// Is this IDCODE value's device id one we know? Used to tell a real DBGMCU
/// read from garbage at a wrong-family address.
pub fn known_st_devid(idcode: u32) -> bool {
    !st_candidates(idcode).is_empty()
}

/// Resolve a Zephyr `CONFIG_SOC` string (e.g. `stm32f401xe`, `nrf52840`,
/// `esp32s3`) to a probe-rs target name. Matching is case-insensitive and a
/// lowercase `x` in the query is a wildcard, which is exactly Zephyr's SoC
/// naming — so the string passes through unchanged.
pub fn resolve_soc(registry: &Registry, soc: &str) -> Option<String> {
    pick_target(registry.search_chips(soc))
}

/// Resolve an STM32 DBGMCU IDCODE value to a probe-rs target name, or `None`
/// if the device id isn't one we know.
pub fn resolve_st_devid(registry: &Registry, idcode: u32) -> Option<String> {
    st_candidates(idcode)
        .iter()
        .find_map(|q| pick_target(registry.search_chips(q)))
}

/// Does the attached chip name agree with the device id read from the silicon?
/// `None` when we can't tell (unknown id, or not an ST chip name) — only a
/// confident `Some(false)` should be surfaced as a mismatch warning.
pub fn st_devid_matches(chip: &str, idcode: u32) -> Option<bool> {
    let candidates = st_candidates(idcode);
    if candidates.is_empty() || !chip.to_ascii_uppercase().starts_with("STM32") {
        return None;
    }
    Some(candidates.iter().any(|q| wildcard_prefix_matches(q, chip)))
}

/// Among registry matches, prefer the largest-flash variant. ST encodes flash
/// size in the last character of the part number (…4=16K, 6=32K, 8=64K, B=128K,
/// C, D, E, F, G, H, I=2M) — monotonically increasing in ASCII — so a max by
/// last byte picks the most permissive memory map. For multi-size device ids
/// the real part can only be ≤ that, and the firmware was built for the real
/// part, so its image always fits.
fn pick_target(mut candidates: Vec<String>) -> Option<String> {
    candidates.sort();
    candidates.dedup();
    candidates.into_iter().max_by_key(|n| n.bytes().last().map(|b| b.to_ascii_uppercase()))
}

/// probe-rs-style prefix match: case-insensitive, lowercase `x` in the query
/// matches any character.
fn wildcard_prefix_matches(query: &str, name: &str) -> bool {
    query.len() <= name.len()
        && query
            .chars()
            .zip(name.chars())
            .all(|(q, n)| q == 'x' || q.eq_ignore_ascii_case(&n))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Registry {
        Registry::from_builtin_families()
    }

    #[test]
    fn zephyr_soc_resolves_to_exact_stm32_target() {
        // Nucleo-F401RE: CONFIG_SOC="stm32f401xe" → an F401 with 512K ("E").
        let name = resolve_soc(&registry(), "stm32f401xe").unwrap();
        assert!(name.to_uppercase().starts_with("STM32F401"), "{name}");
        assert!(name.to_uppercase().ends_with('E'), "{name}");
    }

    #[test]
    fn zephyr_soc_resolves_other_vendors() {
        let nrf = resolve_soc(&registry(), "nrf52840").unwrap();
        assert!(nrf.to_lowercase().contains("nrf52840"), "{nrf}");
        let esp = resolve_soc(&registry(), "esp32s3").unwrap();
        assert!(esp.to_lowercase().starts_with("esp32s3"), "{esp}");
        let rp = resolve_soc(&registry(), "rp2040").unwrap();
        assert!(rp.to_uppercase().starts_with("RP2040"), "{rp}");
    }

    #[test]
    fn unknown_soc_resolves_to_none() {
        assert_eq!(resolve_soc(&registry(), "not-a-chip-9000"), None);
    }

    #[test]
    fn st_devid_resolves_f401() {
        // 0x433 = F401xD/E — what a Nucleo-F401RE reads at 0xE0042000.
        let name = resolve_st_devid(&registry(), 0x1000_6433).unwrap();
        assert!(name.to_uppercase().starts_with("STM32F401"), "{name}");
        assert!(name.to_uppercase().ends_with('E'), "{name}");
    }

    #[test]
    fn st_devid_resolves_h7_and_g0() {
        let h7 = resolve_st_devid(&registry(), 0x450).unwrap();
        assert!(h7.to_uppercase().starts_with("STM32H7"), "{h7}");
        let g0 = resolve_st_devid(&registry(), 0x460).unwrap();
        assert!(g0.to_uppercase().starts_with("STM32G07"), "{g0}");
    }

    #[test]
    fn unknown_devid_resolves_to_none() {
        assert_eq!(resolve_st_devid(&registry(), 0xfff), None);
    }

    #[test]
    fn devid_crosscheck_detects_mismatch_and_match() {
        // Board reads 0x433 (F401xD/E):
        assert_eq!(st_devid_matches("STM32F401RE", 0x433), Some(true));
        assert_eq!(st_devid_matches("STM32H743ZI", 0x433), Some(false));
        // Unknown id or non-ST chip: no opinion.
        assert_eq!(st_devid_matches("STM32F401RE", 0xfff), None);
        assert_eq!(st_devid_matches("nRF52840_xxAA", 0x433), None);
    }

    #[test]
    fn picks_largest_flash_variant() {
        // 0x413 covers F405/407/415/417 in several flash sizes; the pick must
        // be the 1M ("G") variant so any real part's image fits the region.
        let name = resolve_st_devid(&registry(), 0x413).unwrap();
        assert!(name.to_uppercase().ends_with('G'), "{name}");
    }
}
