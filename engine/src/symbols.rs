//! Resolve global symbols from a firmware ELF.
//!
//! This is fully generic: it reads the symbol table of *whatever* ELF is given,
//! so a real application's globals resolve exactly like the demo's. Nothing
//! about any specific firmware is baked in — the host looks up a name, gets its
//! address and size, and reads that live over the probe.

use std::fs;
use std::ops::Range;

use object::{Object, ObjectSection, ObjectSymbol, SectionKind, SymbolKind};

/// A resolved global symbol.
pub struct Symbol {
    pub name: String,
    pub address: u64,
    pub size: u64,
}

/// Look up a symbol by exact name in the ELF's symbol table.
pub fn resolve(elf_path: &str, name: &str) -> Result<Symbol, String> {
    let data = fs::read(elf_path).map_err(|e| format!("read {elf_path}: {e}"))?;
    let file = object::File::parse(&*data).map_err(|e| format!("parse ELF {elf_path}: {e}"))?;
    for sym in file.symbols() {
        if sym.name() == Ok(name) {
            return Ok(Symbol {
                name: name.to_string(),
                address: sym.address(),
                size: sym.size(),
            });
        }
    }
    Err(format!("symbol not found: {name}"))
}

/// List the firmware's global variables (data/bss symbols living in RAM), so an
/// agent can DISCOVER what a project exposes instead of being told. Generic:
/// works on any ELF. Sorted by name, deduplicated.
pub fn list_globals(elf_path: &str) -> Result<Vec<Symbol>, String> {
    let data = fs::read(elf_path).map_err(|e| format!("read {elf_path}: {e}"))?;
    let file = object::File::parse(&*data).map_err(|e| format!("parse ELF {elf_path}: {e}"))?;
    let ram = ram_sections(&file);
    let mut out = Vec::new();
    for sym in file.symbols() {
        if sym.kind() != SymbolKind::Data {
            continue;
        }
        let address = sym.address();
        // Globals live in RAM (.data/.bss-kind sections); skip flash constants,
        // zero-size, and unnamed. Judged against the ELF's own section map, not
        // a hardcoded address range, so any architecture's memory layout works.
        if sym.size() == 0 || !ram.iter().any(|r| r.contains(&address)) {
            continue;
        }
        match sym.name() {
            Ok(name) if !name.is_empty() => out.push(Symbol {
                name: name.to_string(),
                address,
                size: sym.size(),
            }),
            _ => {}
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup_by(|a, b| a.name == b.name);
    Ok(out)
}

/// Address ranges of the ELF's RAM-resident sections (.data/.bss kinds).
fn ram_sections(file: &object::File) -> Vec<Range<u64>> {
    file.sections()
        .filter(|s| matches!(s.kind(), SectionKind::Data | SectionKind::UninitializedData))
        .map(|s| s.address()..s.address() + s.size())
        .filter(|r| r.start < r.end)
        .collect()
}

/// The firmware's RAM address ranges, for sanity-checking raw words that should
/// be pointers into RAM (e.g. thread-name pointers in the trace ring). Derived
/// from the ELF itself — no per-architecture address table.
pub fn ram_ranges(elf_path: &str) -> Result<Vec<Range<u64>>, String> {
    let data = fs::read(elf_path).map_err(|e| format!("read {elf_path}: {e}"))?;
    let file = object::File::parse(&*data).map_err(|e| format!("parse ELF {elf_path}: {e}"))?;
    Ok(ram_sections(&file))
}

/// True when the firmware targets ARM. Gates the decoding paths whose tables
/// are ARM-specific (fatal-error reason names, coredump unwinding) — when the
/// ELF can't be read, says `false` rather than pretend.
pub fn is_arm(elf_path: &str) -> bool {
    let Ok(data) = fs::read(elf_path) else { return false };
    let Ok(file) = object::File::parse(&*data) else { return false };
    file.architecture() == object::Architecture::Arm
}
