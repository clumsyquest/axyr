//! Debug-probe access (SWD) — the foundation for reading the *live* system
//! state and for acting on the board.
//!
//! Unlike the serial path (which only sees what the firmware chooses to print),
//! the debug probe talks straight to the silicon over SWD. On Cortex-M the debug
//! access port can read memory and registers **while the core is running**, so
//! we can observe real state at low overhead — and also flash and reset the
//! board. This is the universal backend that, longer term, works beyond Zephyr.

use std::path::Path;

use probe_rs::flashing::{Format, download_file};
use probe_rs::probe::list::Lister;
use probe_rs::{MemoryInterface, Permissions, Session};

/// The chip on the Nucleo-F401RE. Will become configurable as we add targets.
pub const DEFAULT_CHIP: &str = "STM32F401RETx";

/// A live connection to the target over the first available debug probe.
pub struct Probe {
    session: Session,
}

impl Probe {
    /// Attach to the first probe and the given chip **without resetting or
    /// halting** the core, so we observe the system as it actually runs.
    pub fn attach(chip: &str) -> Result<Self, String> {
        let lister = Lister::new();
        let info = lister
            .list_all()
            .into_iter()
            .next()
            .ok_or("no debug probe found")?;
        let probe = info.open().map_err(|e| format!("open probe: {e}"))?;
        let session = probe
            .attach(chip, Permissions::default())
            .map_err(|e| format!("attach to {chip}: {e}"))?;
        Ok(Self { session })
    }

    /// Read a 32-bit word from target memory. Works while the core runs.
    pub fn read_word(&mut self, address: u64) -> Result<u32, String> {
        let mut core = self.session.core(0).map_err(|e| format!("core: {e}"))?;
        core.read_word_32(address)
            .map_err(|e| format!("read {address:#010x}: {e}"))
    }

    /// Read `out.len()` consecutive 32-bit words starting at `address`.
    pub fn read_words(&mut self, address: u64, out: &mut [u32]) -> Result<(), String> {
        let mut core = self.session.core(0).map_err(|e| format!("core: {e}"))?;
        core.read_32(address, out)
            .map_err(|e| format!("read {} words @ {address:#010x}: {e}", out.len()))
    }

    /// The core's current run/halt status (e.g. Running, Halted).
    pub fn status(&mut self) -> Result<String, String> {
        let mut core = self.session.core(0).map_err(|e| format!("core: {e}"))?;
        let status = core.status().map_err(|e| format!("status: {e}"))?;
        Ok(format!("{status:?}"))
    }

    /// Reset the target and let it run.
    pub fn reset(&mut self) -> Result<(), String> {
        let mut core = self.session.core(0).map_err(|e| format!("core: {e}"))?;
        core.reset().map_err(|e| format!("reset: {e}"))
    }

    /// Flash an ELF image onto the target's flash.
    pub fn flash_elf(&mut self, path: &Path) -> Result<(), String> {
        // Format::default() is ELF with default options.
        download_file(&mut self.session, path, Format::default())
            .map_err(|e| format!("flash {}: {e}", path.display()))
    }
}
