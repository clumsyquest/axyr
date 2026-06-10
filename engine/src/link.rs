//! The probe link — the narrow waist between the analysis engine and the board.
//!
//! Everything the engine knows how to do (crash post-mortem, snapshots,
//! peripheral decode, history…) is built on the few raw primitives below.
//! They carry **no interpretation**: read words, drain telemetry bytes, reset,
//! flash an image. That is the whole point — the side that owns the probe can
//! be a dumb relay with nothing worth reverse-engineering, and the engine can
//! run anywhere (same machine or cloud) without knowing the difference.
//!
//! Two implementations:
//! - [`LocalLink`]: owns the probe in-process (today's all-local mode);
//! - `RemoteLink` (see [`crate::remote`]): the engine end of a WebSocket to a
//!   remote agent that owns the probe.

use std::thread;
use std::time::Duration;

use crate::probe::Probe;
use crate::rtt::Telemetry;

/// Raw board primitives. Implementors are the single owner of their probe; the
/// engine serializes everything through one `&mut dyn ProbeLink`.
pub trait ProbeLink: Send {
    /// Drain available telemetry bytes (RTT up-channel 0) into `out`.
    /// Implementations self-heal a wedged link; an `Err` is transient.
    fn poll_telemetry(&mut self, out: &mut Vec<u8>) -> Result<(), String>;
    /// Re-sync telemetry after the target restarted (reset/flash re-initializes
    /// the RTT control block).
    fn resync_telemetry(&mut self);
    /// Read one 32-bit word. Works while the core runs (no halt).
    fn read_word(&mut self, address: u64) -> Result<u32, String>;
    /// Read `out.len()` consecutive 32-bit words.
    fn read_words(&mut self, address: u64, out: &mut [u32]) -> Result<(), String>;
    /// Read a NUL-terminated string (up to `max` bytes).
    fn read_cstring(&mut self, address: u64, max: usize) -> Result<String, String>;
    /// Read the in-memory coredump buffer at `base`, if a valid dump is there.
    fn read_coredump(&mut self, base: u64) -> Result<Option<Vec<u8>>, String>;
    /// The core's run/halt status (e.g. "Running", "Sleeping", "Halted").
    fn status(&mut self) -> Result<String, String>;
    /// Reset the target and let it run.
    fn reset(&mut self) -> Result<(), String>;
    /// Flash an ELF image (raw bytes) onto the target, then let it run.
    fn flash(&mut self, elf: &[u8]) -> Result<(), String>;
}

/// After this many consecutive failed telemetry cycles, re-open the probe and
/// re-attach RTT (the commodity ST-LINK occasionally wedges).
const SELF_HEAL_AFTER: u32 = 3;

/// The in-process implementation: this process owns the probe.
pub struct LocalLink {
    probe: Probe,
    telemetry: Option<Telemetry>,
    link_errors: u32,
}

impl LocalLink {
    pub fn new(probe: Probe) -> Self {
        Self { probe, telemetry: None, link_errors: 0 }
    }

    /// Access the underlying probe (e.g. for `identity()` at startup).
    pub fn probe_mut(&mut self) -> &mut Probe {
        &mut self.probe
    }

    /// Attach RTT now, blocking until the control block is found (the scan
    /// needs an awake window). Call once at startup for the "RTT attached" log;
    /// otherwise `poll_telemetry` attaches lazily.
    pub fn attach_telemetry_blocking(&mut self) {
        while self.telemetry.is_none() {
            match Telemetry::attach(self.probe.session_mut()) {
                Ok(t) => self.telemetry = Some(t),
                Err(e) => {
                    eprintln!("agent: attach RTT: {e}; retrying...");
                    thread::sleep(Duration::from_millis(300));
                }
            }
        }
        eprintln!("agent: RTT attached; streaming telemetry (no halt)");
    }
}

impl ProbeLink for LocalLink {
    fn poll_telemetry(&mut self, out: &mut Vec<u8>) -> Result<(), String> {
        let Some(telemetry) = self.telemetry.as_mut() else {
            // Not attached yet (e.g. right after a resync): try once, quietly.
            self.telemetry = Telemetry::attach(self.probe.session_mut()).ok();
            return Ok(());
        };

        let mut buf = [0u8; 1024];
        loop {
            match telemetry.read(self.probe.session_mut(), &mut buf) {
                Ok(0) => {
                    self.link_errors = 0;
                    return Ok(());
                }
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(e) => {
                    // Self-heal a wedged SWD link after a few failed cycles:
                    // re-open the probe and re-attach RTT — no human needed.
                    self.link_errors += 1;
                    if self.link_errors >= SELF_HEAL_AFTER {
                        eprintln!("agent: SWD link errors — reattaching...");
                        if self.probe.reattach().is_ok()
                            && let Ok(t) = Telemetry::attach(self.probe.session_mut())
                        {
                            self.telemetry = Some(t);
                            eprintln!("agent: link recovered");
                        }
                        self.link_errors = 0;
                    }
                    return Err(e);
                }
            }
        }
    }

    fn resync_telemetry(&mut self) {
        // Let the firmware re-init RTT, then re-attach to re-sync buffers so we
        // don't miss boot output (e.g. a crash packet right after reset).
        thread::sleep(Duration::from_millis(50));
        match Telemetry::attach(self.probe.session_mut()) {
            Ok(t) => self.telemetry = Some(t),
            Err(e) => {
                eprintln!("agent: RTT re-attach after reset failed: {e}");
                self.telemetry = None; // poll_telemetry keeps retrying lazily
            }
        }
    }

    fn read_word(&mut self, address: u64) -> Result<u32, String> {
        self.probe.read_word(address)
    }

    fn read_words(&mut self, address: u64, out: &mut [u32]) -> Result<(), String> {
        self.probe.read_words(address, out)
    }

    fn read_cstring(&mut self, address: u64, max: usize) -> Result<String, String> {
        self.probe.read_cstring(address, max)
    }

    fn read_coredump(&mut self, base: u64) -> Result<Option<Vec<u8>>, String> {
        self.probe.read_in_memory_coredump(base)
    }

    fn status(&mut self) -> Result<String, String> {
        self.probe.status()
    }

    fn reset(&mut self) -> Result<(), String> {
        self.probe.reset()
    }

    fn flash(&mut self, elf: &[u8]) -> Result<(), String> {
        // probe-rs's loader wants a path; hand it the bytes via a temp file.
        let path = std::env::temp_dir().join(format!("axyr-flash-{}.elf", std::process::id()));
        std::fs::write(&path, elf).map_err(|e| format!("write flash temp: {e}"))?;
        let result = self.probe.flash_elf(&path);
        let _ = std::fs::remove_file(&path);
        result
    }
}
