//! Debug-probe access (SWD) — the foundation for reading the *live* system
//! state and for acting on the board.
//!
//! Unlike the serial path (which only sees what the firmware chooses to print),
//! the debug probe talks straight to the silicon over SWD. On Cortex-M the debug
//! access port can read memory and registers **while the core is running**, so
//! we can observe real state at low overhead — and also flash and reset the
//! board. This is the universal backend that, longer term, works beyond Zephyr.

use std::path::Path;

use std::thread::sleep;
use std::time::Duration;

use probe_rs::flashing::{Format, download_file};
use probe_rs::probe::list::Lister;
use probe_rs::{MemoryInterface, Permissions, Session};

/// Retry a probe operation a few times on transient SWD errors. Commodity
/// ST-LINK clones occasionally throw a spurious "ARM specific error" under load;
/// reads are idempotent, so retrying makes them deterministic.
fn with_retry<T>(mut op: impl FnMut() -> Result<T, String>) -> Result<T, String> {
    let mut last = String::new();
    for attempt in 0..4 {
        match op() {
            Ok(v) => return Ok(v),
            Err(e) => {
                last = e;
                sleep(Duration::from_millis(20 * (attempt + 1)));
            }
        }
    }
    Err(last)
}

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

    /// Mutable access to the underlying session (e.g. for RTT telemetry).
    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }

    /// Read a 32-bit word from target memory. Works while the core runs.
    pub fn read_word(&mut self, address: u64) -> Result<u32, String> {
        let session = &mut self.session;
        with_retry(|| {
            let mut core = session.core(0).map_err(|e| format!("core: {e}"))?;
            core.read_word_32(address)
                .map_err(|e| format!("read {address:#010x}: {e}"))
        })
    }

    /// Read `out.len()` consecutive 32-bit words starting at `address`.
    pub fn read_words(&mut self, address: u64, out: &mut [u32]) -> Result<(), String> {
        let session = &mut self.session;
        with_retry(|| {
            let mut core = session.core(0).map_err(|e| format!("core: {e}"))?;
            core.read_32(address, out)
                .map_err(|e| format!("read {} words @ {address:#010x}: {e}", out.len()))
        })
    }

    /// Read a Zephyr in-memory coredump straight from the target's RAM buffer.
    ///
    /// The IN_MEMORY backend lays the buffer out as
    /// `[canary 4B][size u32][raw coredump ...][canary]`, so we read the whole
    /// dump in one block over SWD (the core is halted after a fault, so SRAM
    /// reads work) — no log subsystem, no hex, no streaming. `base` is the
    /// address of the `in_memory_coredump` symbol. Returns `None` if the canary
    /// shows no valid dump.
    pub fn read_in_memory_coredump(&mut self, base: u64) -> Result<Option<Vec<u8>>, String> {
        const CANARY: [u8; 4] = [0xDE, 0xB0, 0xDE, 0xB0];
        let mut core = self.session.core(0).map_err(|e| format!("core: {e}"))?;

        let mut canary = [0u8; 4];
        core.read(base, &mut canary)
            .map_err(|e| format!("read coredump canary: {e}"))?;
        if canary != CANARY {
            return Ok(None); // no valid dump stored
        }

        let size = core
            .read_word_32(base + 4)
            .map_err(|e| format!("read coredump size: {e}"))? as usize;
        if size == 0 || size > 256 * 1024 {
            return Ok(None); // implausible size
        }

        let mut data = vec![0u8; size];
        core.read(base + 8, &mut data)
            .map_err(|e| format!("read coredump data: {e}"))?;
        Ok(Some(data))
    }

    /// Read a NUL-terminated string from target memory (up to `max` bytes).
    pub fn read_cstring(&mut self, address: u64, max: usize) -> Result<String, String> {
        let session = &mut self.session;
        with_retry(|| {
            let mut core = session.core(0).map_err(|e| format!("core: {e}"))?;
            let mut buf = vec![0u8; max];
            core.read(address, &mut buf)
                .map_err(|e| format!("read string @ {address:#010x}: {e}"))?;
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            Ok(String::from_utf8_lossy(&buf[..end]).to_string())
        })
    }

    /// The core's current run/halt status (e.g. Running, Halted).
    pub fn status(&mut self) -> Result<String, String> {
        let mut core = self.session.core(0).map_err(|e| format!("core: {e}"))?;
        let status = core.status().map_err(|e| format!("status: {e}"))?;
        Ok(format!("{status:?}"))
    }

    /// Reset the target and let it run.
    pub fn reset(&mut self) -> Result<(), String> {
        let session = &mut self.session;
        with_retry(|| {
            let mut core = session.core(0).map_err(|e| format!("core: {e}"))?;
            core.reset().map_err(|e| format!("reset: {e}"))
        })
    }

    /// Flash an ELF image onto the target's flash.
    pub fn flash_elf(&mut self, path: &Path) -> Result<(), String> {
        // Format::default() is ELF with default options.
        download_file(&mut self.session, path, Format::default())
            .map_err(|e| format!("flash {}: {e}", path.display()))
    }
}
