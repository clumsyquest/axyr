//! Decode a peripheral's live register state into plain language, using the
//! chip's SVD (CMSIS System View Description).
//!
//! Given a peripheral name, we look up its registers and bit-fields in the SVD,
//! read the (readable) registers over the probe, and render each field with its
//! value and — when the SVD defines one — its symbolic meaning. This turns raw
//! hex into "what the hardware is doing" (e.g. UART enabled, a GPIO pin HIGH).
//!
//! Non-intrusive by construction: registers whose read has a side effect
//! (`readAction`, e.g. a data register that pops a FIFO) or that are write-only
//! are SKIPPED — reading must never disturb the running system.

use svd_parser::svd::Device;

/// Parse an SVD file into a `Device`. Call once and cache; `expand` resolves
/// `derivedFrom` and register arrays so every peripheral has its registers.
pub fn load_svd(path: &str) -> Result<Device, String> {
    let xml = std::fs::read_to_string(path).map_err(|e| format!("read SVD {path}: {e}"))?;
    let config = svd_parser::Config::default().expand(true);
    svd_parser::parse_with_config(&xml, &config).map_err(|e| format!("parse SVD: {e}"))
}

/// Decode a peripheral's live state. `read(addr)` reads a 32-bit word over the
/// probe. Only side-effect-free, readable registers are touched.
pub fn decode<R>(device: &Device, name: &str, mut read: R) -> Result<String, String>
where
    R: FnMut(u64) -> Result<u32, String>,
{
    let periph = device
        .peripherals
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| format!("peripheral not found in SVD: {name}"))?;

    let base = periph.base_address;
    let mut out = format!("{} @{base:#010x}\n", periph.name);

    for reg in periph.all_registers() {
        // Non-intrusive: skip registers whose read has side effects, or that
        // cannot be read at all.
        if reg.read_action.is_some() {
            continue;
        }
        if let Some(access) = reg.properties.access
            && !access.can_read()
        {
            continue;
        }

        let addr = base + reg.address_offset as u64;
        let Ok(value) = read(addr) else { continue };
        out.push_str(&format!("  {:<10} = {value:#010x}\n", reg.name));

        for field in reg.fields() {
            let width = field.bit_range.width;
            let mask = if width >= 32 { u32::MAX } else { (1u32 << width) - 1 };
            let fval = (value >> field.bit_range.offset) & mask;

            // Symbolic meaning from the SVD's enumerated values, if any.
            let meaning = field
                .enumerated_values
                .iter()
                .flat_map(|ev| ev.values.iter())
                .find(|e| e.value == Some(fval as u64))
                .map(|e| format!(" ({})", e.name));

            // Keep it readable: show fields that are set or have a named meaning.
            if fval != 0 || meaning.is_some() {
                out.push_str(&format!(
                    "      {:<12} = {fval}{}\n",
                    field.name,
                    meaning.unwrap_or_default()
                ));
            }
        }
    }
    Ok(out.trim_end().to_string())
}
