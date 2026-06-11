//! The serial transport — talk to a board over its plain USB-UART, no debug
//! probe required.
//!
//! This is the zero-hardware path: every dev board already has a USB serial
//! port (ESP32's CP210x/CH340 bridge, a Nucleo's VCP, …). Over it the engine
//! gets the firmware's telemetry (so the crash line, threads, trace markers),
//! the full coredump when the firmware prints it as `#CD:` lines (Zephyr's
//! logging backend — the dump carries its own stack memory, so the call stack
//! resolves with no memory reads at all), reset (the DTR/RTS pulse esptool
//! uses), and flashing via the `espflash` tool when it's installed.
//!
//! What serial honestly CANNOT do: read arbitrary memory of a *running* core.
//! Those primitives return a clear error instead of a guess — live variables,
//! register decode and the in-memory coredump need a debug probe (SWD/JTAG),
//! or the on-device sampler (roadmap) which pushes values over this same port.

use std::io::Read;
use std::process::Command;
use std::thread;
use std::time::Duration;

use serialport::{SerialPort, SerialPortType};

use crate::link::ProbeLink;

/// USB vendor ids of the common UART bridges (and Espressif's native USB):
/// Silabs CP210x, WCH CH340, FTDI, Espressif.
const UART_BRIDGE_VIDS: &[u16] = &[0x10C4, 0x1A86, 0x0403, 0x303A];

const NOT_OVER_SERIAL: &str =
    "not available over serial — live memory access needs a debug probe (SWD/JTAG)";

/// Pick the board's serial port: `AXYR_SERIAL` wins; otherwise the first port
/// whose USB id is a known UART bridge; otherwise the only USB port present.
/// Several unknown ports is genuine ambiguity — guessing could point reset()
/// and espflash at someone else's board, so name the candidates and decline.
pub fn detect_port() -> Option<String> {
    if let Ok(p) = std::env::var("AXYR_SERIAL") {
        return Some(p);
    }
    let ports = serialport::available_ports().ok()?;
    let mut usb: Vec<_> = ports
        .iter()
        .filter_map(|p| match &p.port_type {
            SerialPortType::UsbPort(info) => Some((p.port_name.clone(), info.vid)),
            _ => None,
        })
        .collect();
    // macOS lists every adapter twice (/dev/cu.X and /dev/tty.X); keep cu.*,
    // the call-out node meant for a host that initiates.
    let cu: Vec<String> = usb
        .iter()
        .filter_map(|(n, _)| n.strip_prefix("/dev/cu.").map(str::to_string))
        .collect();
    usb.retain(|(n, _)| n.strip_prefix("/dev/tty.").is_none_or(|s| !cu.contains(&s.to_string())));

    if let Some((name, _)) = usb.iter().find(|(_, vid)| UART_BRIDGE_VIDS.contains(vid)) {
        return Some(name.clone());
    }
    match usb.as_slice() {
        [(only, _)] => Some(only.clone()),
        [] => None,
        many => {
            eprintln!("axyr: several USB serial ports and none is a known UART bridge:");
            for (name, vid) in many {
                eprintln!("  {name} (vid {vid:#06x})");
            }
            eprintln!("  pick one explicitly:  AXYR_SERIAL=<port> axyr");
            None
        }
    }
}

/// A board reached over its USB serial port. Implements the same narrow waist
/// as the probe ([`ProbeLink`]); the engine above does not care.
pub struct SerialLink {
    port: Option<Box<dyn SerialPort>>,
    path: String,
    baud: u32,
}

impl SerialLink {
    /// Open `path` (e.g. `/dev/ttyUSB0`, `COM5`). Baud from `AXYR_BAUD`,
    /// default 115200 (Zephyr's console default).
    pub fn open(path: &str) -> Result<Self, String> {
        let baud = std::env::var("AXYR_BAUD")
            .ok()
            .and_then(|b| b.parse().ok())
            .unwrap_or(115_200);
        let port = open_port(path, baud)?;
        Ok(Self { port: Some(port), path: path.to_string(), baud })
    }

    pub fn path(&self) -> &str {
        &self.path
    }
}

fn open_port(path: &str, baud: u32) -> Result<Box<dyn SerialPort>, String> {
    serialport::new(path, baud)
        .timeout(Duration::from_millis(20))
        .open()
        .map_err(|e| format!("open serial {path}: {e}"))
}

impl ProbeLink for SerialLink {
    fn poll_telemetry(&mut self, out: &mut Vec<u8>) -> Result<(), String> {
        // A chatty device never leaves the 20ms gap a timeout needs, so an
        // "until empty" drain could spin forever; cap each cycle and let the
        // agent come back for the rest (the OS buffers the remainder).
        const MAX_DRAIN: usize = 64 * 1024;
        let Some(port) = self.port.as_mut() else {
            // Re-open after flash/reset released the port, or after a read
            // error dropped it (replug usually restores the same name).
            self.port = Some(open_port(&self.path, self.baud)?);
            return Ok(());
        };
        let mut buf = [0u8; 1024];
        let start = out.len();
        let err = loop {
            match port.read(&mut buf) {
                // read() only runs once poll() reports readiness, so zero
                // bytes is EOF — the device is gone. (A quiet board is
                // Err(TimedOut), never Ok(0).)
                Ok(0) => break "serial EOF — device disconnected".to_string(),
                Ok(n) => {
                    out.extend_from_slice(&buf[..n]);
                    if out.len() - start >= MAX_DRAIN {
                        return Ok(());
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => return Ok(()),
                Err(e) => break format!("serial read: {e}"),
            }
        };
        // The handle is dead (unplug, re-enumeration). Drop it so the next
        // cycle re-opens — the ProbeLink contract says an Err is transient.
        self.port = None;
        Err(err)
    }

    fn resync_telemetry(&mut self) {
        // Serial survives a target reset; just drop whatever half-line the
        // reset interrupted.
        if let Some(port) = self.port.as_mut() {
            let _ = port.clear(serialport::ClearBuffer::Input);
        }
    }

    fn read_word(&mut self, _address: u64) -> Result<u32, String> {
        Err(NOT_OVER_SERIAL.to_string())
    }

    fn read_words(&mut self, _address: u64, _out: &mut [u32]) -> Result<(), String> {
        Err(NOT_OVER_SERIAL.to_string())
    }

    fn read_cstring(&mut self, _address: u64, _max: usize) -> Result<String, String> {
        Err(NOT_OVER_SERIAL.to_string())
    }

    fn read_coredump(&mut self, _base: u64) -> Result<Option<Vec<u8>>, String> {
        // The serial path gets its coredump from the `#CD:` telemetry block
        // instead (the agent collects it); there is no memory to read here.
        Err(NOT_OVER_SERIAL.to_string())
    }

    fn status(&mut self) -> Result<String, String> {
        // No debug port — we cannot see run/halt. Say so rather than guess.
        Ok("Unknown (serial — no debug probe)".to_string())
    }

    fn reset(&mut self) -> Result<(), String> {
        // The esptool hard reset: the dev board wires RTS→EN and DTR→IO0
        // through the auto-program transistors. Pulse EN low with IO0
        // released and the chip reboots into the application. Every RTS
        // change is followed by a dummy DTR write: Windows' usbser.sys only
        // transmits the new control-line state on a DTR write (esptool's
        // workaround), and native-USB ESP32s (VID 0x303A) bind that driver.
        {
            let port = self.port.as_mut().ok_or("serial port not open")?;
            let dtr = |p: &mut Box<dyn SerialPort>| {
                p.write_data_terminal_ready(false).map_err(|e| format!("reset (DTR): {e}"))
            };
            dtr(port)?;
            port.write_request_to_send(true).map_err(|e| format!("reset (RTS): {e}"))?;
            dtr(port)?;
            thread::sleep(Duration::from_millis(200));
            port.write_request_to_send(false).map_err(|e| format!("reset (RTS): {e}"))?;
            dtr(port)?;
        }
        // A native-USB chip re-enumerates on reset, killing the open handle.
        // Drop it (poll_telemetry re-opens lazily) and give the chip time to
        // come out of reset — esptool waits the same 200ms.
        self.port = None;
        thread::sleep(Duration::from_millis(200));
        Ok(())
    }

    fn flash(&mut self, elf: &[u8]) -> Result<(), String> {
        // Flashing over serial speaks the chip's ROM bootloader. `espflash`
        // implements it for the whole ESP32 family; use it when installed
        // rather than reimplementing the protocol here. (espflash packages
        // the ELF in its esp-idf app format — right for ESP-IDF/esp-hal
        // builds; a Zephyr image that needs its own bootloader/partition
        // layout should go through `west flash` until this is verified.)
        let tmp = std::env::temp_dir().join(format!("axyr-serial-flash-{}.elf", std::process::id()));
        std::fs::write(&tmp, elf).map_err(|e| format!("write flash temp: {e}"))?;
        self.port = None; // espflash needs the port to itself
        let result = Command::new("espflash")
            .args(["flash", "--port", &self.path])
            .arg(&tmp)
            .output();
        let _ = std::fs::remove_file(&tmp);
        let out = result.map_err(|_| {
            "flashing over serial uses the espflash tool and it was not found — \
             install it (https://github.com/esp-rs/espflash) or use a debug probe"
                .to_string()
        })?;
        // poll_telemetry lazily re-opens the port on the next cycle.
        if !out.status.success() {
            return Err(format!(
                "espflash failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(())
    }
}
