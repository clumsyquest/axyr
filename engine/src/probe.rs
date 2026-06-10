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

use probe_rs::config::{Registry, TargetSelector};
use probe_rs::flashing::{Format, download_file};
use probe_rs::probe::list::Lister;
use probe_rs::{MemoryInterface, Permissions, Session};

use crate::chip;

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

/// How [`Probe::attach_auto`] identified the chip — shown on the Connect
/// screen and in the startup banner, so detection is never a silent guess.
pub struct Detection {
    /// The probe-rs target we attached to (e.g. `STM32F401RE`).
    pub chip: String,
    /// Human-readable source of the decision (build, silicon, fallback…).
    pub method: String,
    /// `true` when we fell back to a generic Cortex-M target: observation
    /// works, but flashing needs the real chip (`AXYR_CHIP`).
    pub generic: bool,
}

/// A live connection to the target over the first available debug probe.
pub struct Probe {
    session: Session,
    chip: String,
}

impl Probe {
    /// Identify the first connected debug probe (name, serial) **without
    /// attaching** — for the dashboard's "board detected" Connect screen.
    pub fn list_first() -> Option<(String, Option<String>)> {
        Lister::new()
            .list_all()
            .into_iter()
            .next()
            .map(|i| (i.identifier.clone(), i.serial_number.clone()))
    }

    /// Read the chip identity straight from silicon: the Cortex-M CPUID and, on
    /// STM32, the DBGMCU IDCODE (its low 12 bits are the device id, e.g. 0x433
    /// for the STM32F401). The IDCODE address varies per STM32 family, so this
    /// sweeps the known locations and keeps the first one holding a device id
    /// we recognize. Either is `None` if the read isn't supported.
    pub fn identity(&mut self) -> (Option<u32>, Option<u32>) {
        let cpuid = self.read_word(0xE000_ED00).ok();
        let idcode = chip::ST_DBGMCU_ADDRS
            .iter()
            .filter_map(|&a| self.read_word_once(a).ok())
            .find(|&v| chip::known_st_devid(v));
        (cpuid, idcode)
    }

    /// The probe-rs target name this session is attached to.
    pub fn chip(&self) -> &str {
        &self.chip
    }

    /// Attach to the first probe and the given chip **without resetting or
    /// halting** the core, so we observe the system as it actually runs.
    pub fn attach(chip: &str) -> Result<Self, String> {
        Self::attach_selector(chip.into())
    }

    fn attach_selector(target: TargetSelector) -> Result<Self, String> {
        let label = match &target {
            TargetSelector::Unspecified(name) => name.clone(),
            _ => "auto-detected chip".to_string(),
        };
        let info = Lister::new()
            .list_all()
            .into_iter()
            .next()
            .ok_or("no debug probe found")?;
        let probe = info.open().map_err(|e| format!("open probe: {e}"))?;
        let session = probe
            .attach(target, Permissions::default())
            .map_err(|e| format!("attach to {label}: {e}"))?;
        // Keep the registry's canonical name (what Auto resolved to).
        let chip = session.target().name.clone();
        Ok(Self { session, chip })
    }

    /// Attach without being told the chip — the layered auto-detection that
    /// makes `axyr` work on whatever board is plugged in:
    ///
    /// 1. the firmware build's `CONFIG_SOC` (exact, instant, cross-vendor);
    /// 2. probe-rs's silicon auto-detect (Nordic, Espressif, Infineon, …);
    /// 3. our ST detection: attach a generic Cortex-M, read the DBGMCU device
    ///    id, re-attach the precise target (probe-rs can't auto-detect ST);
    /// 4. stay on the generic target — observation only, but never a dead end.
    ///
    /// Every layer is non-intrusive: no reset, no halt, only debug-port reads.
    pub fn attach_auto(soc: Option<&str>) -> Result<(Self, Detection), String> {
        let registry = Registry::from_builtin_families();

        // 1) The firmware build names the SoC it was compiled for.
        if let Some(soc) = soc {
            if let Some(name) = chip::resolve_soc(&registry, soc) {
                match Self::attach(&name) {
                    Ok(p) => {
                        let chip = p.chip.clone();
                        return Ok((p, Detection {
                            chip,
                            method: format!("firmware build (CONFIG_SOC={soc})"),
                            generic: false,
                        }));
                    }
                    Err(e) => eprintln!(
                        "axyr: build says {name} but attach failed ({e}); probing the silicon instead"
                    ),
                }
            } else {
                eprintln!("axyr: CONFIG_SOC={soc} is not in the target registry; probing the silicon instead");
            }
        }

        // 2) probe-rs's own silicon detection (vendors with detect sequences).
        if let Ok(p) = Self::attach_selector(TargetSelector::Auto) {
            let chip = p.chip.clone();
            return Ok((p, Detection {
                chip,
                method: "silicon (probe-rs auto-detect)".to_string(),
                generic: false,
            }));
        }

        // 3+4) Generic attach, then identify ST silicon by its device id.
        for generic in chip::CORTEX_GENERICS {
            let Ok(mut p) = Self::attach(generic) else { continue };
            if p.read_word_once(0xE000_ED00).is_err() {
                continue; // wrong architecture guess — try the next core type
            }
            for &addr in chip::ST_DBGMCU_ADDRS {
                let Ok(idcode) = p.read_word_once(addr) else { continue };
                if let Some(name) = chip::resolve_st_devid(&registry, idcode) {
                    drop(p); // release the probe before the precise re-attach
                    let p = Self::attach(&name)?;
                    return Ok((p, Detection {
                        chip: name,
                        method: format!("silicon (ST device id {:#05x})", idcode & 0xfff),
                        generic: false,
                    }));
                }
            }
            let chip = p.chip.clone();
            return Ok((p, Detection {
                chip,
                method: format!("fallback (unknown chip, generic {generic})"),
                generic: true,
            }));
        }

        Err("no debug probe found, or could not attach to the target".to_string())
    }

    /// Re-open the probe and re-attach the session — recovers a wedged SWD link
    /// without a human (the commodity ST-LINK occasionally needs this).
    pub fn reattach(&mut self) -> Result<(), String> {
        let info = Lister::new()
            .list_all()
            .into_iter()
            .next()
            .ok_or("no debug probe found")?;
        let probe = info.open().map_err(|e| format!("open probe: {e}"))?;
        self.session = probe
            .attach(&self.chip, Permissions::default())
            .map_err(|e| format!("re-attach to {}: {e}", self.chip))?;
        Ok(())
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

    /// Single-attempt word read — for detection sweeps, where a failed read at
    /// a candidate address is an expected (and fast) "not this one".
    fn read_word_once(&mut self, address: u64) -> Result<u32, String> {
        let mut core = self.session.core(0).map_err(|e| format!("core: {e}"))?;
        core.read_word_32(address)
            .map_err(|e| format!("read {address:#010x}: {e}"))
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
