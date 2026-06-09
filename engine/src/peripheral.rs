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

use serde_json::{Value, json};
use svd_parser::svd::Device;

/// A decoded field within a register.
pub struct FieldState {
    pub name: String,
    pub value: u32,
    pub meaning: Option<String>,
}

/// A decoded register: its raw value plus the set/meaningful fields.
pub struct RegState {
    pub name: String,
    pub value: u32,
    pub fields: Vec<FieldState>,
}

/// Read a peripheral's readable registers and decode their fields (structured).
/// Side-effect / write-only registers are skipped (non-intrusive).
pub fn read_state<R>(device: &Device, name: &str, mut read: R) -> Result<Vec<RegState>, String>
where
    R: FnMut(u64) -> Result<u32, String>,
{
    let periph = device
        .peripherals
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| format!("peripheral not found in SVD: {name}"))?;
    let base = periph.base_address;

    let mut regs = Vec::new();
    for reg in periph.all_registers() {
        if reg.read_action.is_some() {
            continue;
        }
        if let Some(access) = reg.properties.access
            && !access.can_read()
        {
            continue;
        }
        let Ok(value) = read(base + reg.address_offset as u64) else { continue };

        let mut fields = Vec::new();
        for field in reg.fields() {
            let width = field.bit_range.width;
            let mask = if width >= 32 { u32::MAX } else { (1u32 << width) - 1 };
            let fval = (value >> field.bit_range.offset) & mask;
            let meaning = field
                .enumerated_values
                .iter()
                .flat_map(|ev| ev.values.iter())
                .find(|e| e.value == Some(fval as u64))
                .map(|e| e.name.clone());
            if fval != 0 || meaning.is_some() {
                fields.push(FieldState { name: field.name.clone(), value: fval, meaning });
            }
        }
        regs.push(RegState { name: reg.name.clone(), value, fields });
    }
    Ok(regs)
}

/// Structured peripheral state as JSON (for the snapshot contract).
pub fn to_json(name: &str, base: u64, regs: &[RegState]) -> Value {
    json!({
        "name": name,
        "address": format!("{base:#010x}"),
        "registers": regs.iter().map(|r| json!({
            "name": r.name,
            "value": format!("{:#010x}", r.value),
            "fields": r.fields.iter().map(|f| json!({
                "name": f.name, "value": f.value, "meaning": f.meaning
            })).collect::<Vec<_>>()
        })).collect::<Vec<_>>()
    })
}

/// Parse an SVD file into a `Device`. Call once and cache; `expand` resolves
/// `derivedFrom` and register arrays so every peripheral has its registers.
pub fn load_svd(path: &str) -> Result<Device, String> {
    let xml = std::fs::read_to_string(path).map_err(|e| format!("read SVD {path}: {e}"))?;
    let config = svd_parser::Config::default().expand(true);
    svd_parser::parse_with_config(&xml, &config).map_err(|e| format!("parse SVD: {e}"))
}

/// Decode a peripheral's live state. `read(addr)` reads a 32-bit word over the
/// probe. Only side-effect-free, readable registers are touched.
pub fn decode<R>(device: &Device, name: &str, read: R) -> Result<String, String>
where
    R: FnMut(u64) -> Result<u32, String>,
{
    let regs = read_state(device, name, read)?;
    let mut out = format!("{}\n", name.to_uppercase());
    for r in &regs {
        out.push_str(&format!("  {:<10} = {:#010x}\n", r.name, r.value));
        for f in &r.fields {
            let meaning = f.meaning.as_ref().map(|m| format!(" ({m})")).unwrap_or_default();
            out.push_str(&format!("      {:<12} = {}{meaning}\n", f.name, f.value));
        }
    }
    Ok(out.trim_end().to_string())
}
