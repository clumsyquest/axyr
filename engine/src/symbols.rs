//! Resolve global symbols from a firmware ELF.
//!
//! This is fully generic: it reads the symbol table of *whatever* ELF is given,
//! so a real application's globals resolve exactly like the demo's. Nothing
//! about any specific firmware is baked in — the host looks up a name, gets its
//! address and size, and reads that live over the probe.

use std::fs;

use object::{Object, ObjectSymbol, SymbolKind};

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
    let mut out = Vec::new();
    for sym in file.symbols() {
        if sym.kind() != SymbolKind::Data {
            continue;
        }
        let address = sym.address();
        // Globals live in RAM; skip flash constants, zero-size, and unnamed.
        if !(0x2000_0000..0x2002_0000).contains(&address) || sym.size() == 0 {
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
