//! Resolve global symbols from a firmware ELF.
//!
//! This is fully generic: it reads the symbol table of *whatever* ELF is given,
//! so a real application's globals resolve exactly like the demo's. Nothing
//! about any specific firmware is baked in — the host looks up a name, gets its
//! address and size, and reads that live over the probe.

use std::fs;

use object::{Object, ObjectSymbol};

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
