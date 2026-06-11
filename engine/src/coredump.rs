//! Coredump capture and call-stack resolution.
//!
//! When `CONFIG_DEBUG_COREDUMP=y`, Zephyr emits a full CPU snapshot over the
//! serial line as a block of `#CD:` log lines, framed by `#CD:BEGIN#` and
//! `#CD:END#`. This module:
//!   1. collects that block from the serial stream ([`CoredumpCollector`]), then
//!   2. turns it into a human-readable call stack ([`resolve_backtrace`]) by
//!      driving Zephyr's own coredump tooling and GDB offline.
//!
//! The heavy lifting (parsing the binary dump, unwinding the stack) is done by
//! the toolchain that already ships with Zephyr; we only orchestrate it.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

const BEGIN_MARKER: &str = "#CD:BEGIN#";
const END_MARKER: &str = "#CD:END#";
const PREFIX: &str = "#CD:";

/// Strip ANSI escape sequences (e.g. the color codes Zephyr's log backend adds)
/// from a line. The serial coredump lines are wrapped in `\x1b[..m ... \x1b[0m`,
/// and the trailing reset code would otherwise corrupt the hex payload.
pub fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip until the terminating letter of the escape sequence.
            for e in chars.by_ref() {
                if e.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Is this serial line coredump plumbing (a `#CD:` line)? The agent keeps
/// these out of the human-facing recent-log — hundreds of hex lines would
/// evict the application output the crash report exists to show.
pub fn is_dump_line(line: &str) -> bool {
    strip_ansi(line).contains(PREFIX)
}

/// Does this line start a new `#CD:` dump? The agent uses this to stop
/// waiting for a dump that never completed: a new dump beginning while an
/// older crash still waits means that crash's dump is gone for good.
pub fn is_dump_begin(line: &str) -> bool {
    strip_ansi(line).contains(BEGIN_MARKER)
}

/// Accumulates the `#CD:` coredump block as serial lines stream in.
///
/// Feed every serial line through [`feed`](Self::feed); it returns the captured
/// block (the cleaned `#CD:` lines, newline-joined) once `#CD:END#` is seen.
#[derive(Default)]
pub struct CoredumpCollector {
    capturing: bool,
    lines: Vec<String>,
}

impl CoredumpCollector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Process one serial line. Returns `Some(block)` when a complete coredump
    /// (BEGIN..END) has been captured, otherwise `None`.
    pub fn feed(&mut self, line: &str) -> Option<String> {
        let line = strip_ansi(line);

        if line.contains(BEGIN_MARKER) {
            // A new dump starts; drop anything half-captured before.
            self.capturing = true;
            self.lines.clear();
        }

        if !self.capturing {
            return None;
        }

        // Keep only the coredump lines themselves, trimmed to start at "#CD:".
        if let Some(idx) = line.find(PREFIX) {
            self.lines.push(line[idx..].to_string());
        }

        if line.contains(END_MARKER) {
            self.capturing = false;
            let block = self.lines.join("\n");
            self.lines.clear();
            return Some(block);
        }

        None
    }
}

/// Decode a captured `#CD:` block (the string [`CoredumpCollector::feed`]
/// returns) into the raw coredump bytes — the same `ZE…` binary the IN_MEMORY
/// backend holds in RAM. Zephyr's log backend emits the dump as hex on `#CD:`
/// lines; BEGIN/END/ERROR marker lines carry no payload and are skipped.
pub fn block_to_bytes(block: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    for line in block.lines() {
        let Some(payload) = line.trim().strip_prefix(PREFIX) else { continue };
        // "#CD:ERROR CANNOT DUMP#": the firmware aborted mid-dump — bytes are
        // missing and every later offset is shifted, so decoding the rest
        // would fabricate a plausible-but-wrong dump. Refuse instead.
        if payload.contains("ERROR") {
            return Err("firmware reported an incomplete dump (#CD:ERROR)".to_string());
        }
        // Markers: "BEGIN#", "END#", …
        if payload.ends_with('#') {
            continue;
        }
        let hex: String = payload.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        if hex.len() != payload.trim().len() {
            return Err(format!("coredump line has non-hex payload: {line}"));
        }
        // Zephyr emits exactly two hex chars per byte; an odd length means the
        // UART lost a character and everything after it is byte-shifted.
        if !hex.len().is_multiple_of(2) {
            return Err(format!("coredump line has odd hex length: {line}"));
        }
        for i in (0..hex.len()).step_by(2) {
            let byte = u8::from_str_radix(&hex[i..i + 2], 16)
                .map_err(|e| format!("coredump hex decode: {e}"))?;
            out.push(byte);
        }
    }
    if out.is_empty() {
        return Err("coredump block has no payload".to_string());
    }
    Ok(out)
}

/// Paths to the external tools needed to turn a coredump into a backtrace.
///
/// All of these ship with a Zephyr install + SDK; the engine only shells out to
/// them. Built from the environment via [`from_env`](Self::from_env) so the
/// binary stays free of hard-coded machine paths.
pub struct CoredumpTools {
    pub python: String,
    pub gdb: String,
    /// Zephyr's `coredump_serial_log_parser.py`.
    pub log_parser: String,
    /// Zephyr's `coredump_gdbserver.py`.
    pub gdbserver: String,
    /// The firmware ELF, for symbol resolution.
    pub elf: String,
}

impl CoredumpTools {
    /// Build the tool set from environment variables, given the ELF path.
    /// Returns `None` if any required tool path is missing, so the caller can
    /// degrade gracefully to a backtrace-less report.
    ///
    ///   AXYR_PYTHON              (optional, defaults to "python3")
    ///   AXYR_GDB                 path to arm-zephyr-eabi-gdb
    ///   AXYR_COREDUMP_LOG_PARSER path to coredump_serial_log_parser.py
    ///   AXYR_COREDUMP_GDBSERVER  path to coredump_gdbserver.py
    pub fn from_env(elf: &str) -> Option<Self> {
        Some(Self {
            python: std::env::var("AXYR_PYTHON").unwrap_or_else(|_| "python3".to_string()),
            gdb: std::env::var("AXYR_GDB").ok()?,
            log_parser: std::env::var("AXYR_COREDUMP_LOG_PARSER").ok()?,
            gdbserver: std::env::var("AXYR_COREDUMP_GDBSERVER").ok()?,
            elf: elf.to_string(),
        })
    }
}

/// Turn a captured coredump block into a human-readable call stack.
///
/// Pipeline: write the block to a temp log → Zephyr's parser produces a binary
/// coredump → GDB unwinds it offline (the parser doubles as a GDB remote target
/// in `--pipe` mode) → we keep the `bt` frames. Returns an error string on any
/// failure so the caller can fall back to the fast (location-only) report.
pub fn resolve_backtrace(tools: &CoredumpTools, block: &str) -> Result<String, String> {
    let tmp = std::env::temp_dir();
    let pid = std::process::id();
    let log_path: PathBuf = tmp.join(format!("axyr-coredump-{pid}.log"));
    let bin_path: PathBuf = tmp.join(format!("axyr-coredump-{pid}.bin"));

    fs::write(&log_path, block).map_err(|e| format!("write coredump log: {e}"))?;

    // 1. Parse the serial log into a binary coredump.
    let parse = Command::new(&tools.python)
        .args([&tools.log_parser, path(&log_path), path(&bin_path)])
        .output()
        .map_err(|e| format!("run coredump parser: {e}"))?;
    if !parse.status.success() {
        return Err(format!(
            "coredump parser failed: {}",
            String::from_utf8_lossy(&parse.stderr).trim()
        ));
    }

    // 2. Drive GDB offline on the parsed binary coredump.
    run_gdb_on(tools, &bin_path)
}

/// Resolve a backtrace directly from a raw binary coredump (e.g. read from the
/// target's IN_MEMORY backend over SWD), skipping the serial-log hex parser.
pub fn resolve_backtrace_from_bin(tools: &CoredumpTools, coredump: &[u8]) -> Result<String, String> {
    let bin_path =
        std::env::temp_dir().join(format!("axyr-coredump-{}.bin", std::process::id()));
    fs::write(&bin_path, coredump).map_err(|e| format!("write coredump bin: {e}"))?;
    run_gdb_on(tools, &bin_path)
}

/// Drive GDB offline over a pipe: the gdbserver script serves the binary
/// coredump as a remote target; keep the `bt` frames.
fn run_gdb_on(tools: &CoredumpTools, bin_path: &std::path::Path) -> Result<String, String> {
    let remote = format!(
        "target remote | {} {} --pipe {} {}",
        tools.python, tools.gdbserver, tools.elf, path(bin_path)
    );
    let gdb = Command::new(&tools.gdb)
        .args(["-q", "-batch", &tools.elf, "-ex", &remote, "-ex", "bt"])
        .output()
        .map_err(|e| format!("run gdb: {e}"))?;
    if !gdb.status.success() {
        return Err(format!(
            "gdb failed: {}",
            String::from_utf8_lossy(&gdb.stderr).trim()
        ));
    }

    let backtrace = format_backtrace(&String::from_utf8_lossy(&gdb.stdout));
    if backtrace.is_empty() {
        return Err("gdb produced no stack frames".to_string());
    }
    Ok(backtrace)
}

/// Extract and clean GDB `bt` output into a compact call stack.
///
/// GDB prints frames like:
///   `#0  0x0800046a in i2c_read_reg (reg=0) at .../main.c:30`
/// We keep one tidy line per frame: `#0 i2c_read_reg at .../main.c:30`.
pub fn format_backtrace(gdb_output: &str) -> String {
    let mut frames = Vec::new();
    for line in gdb_output.lines() {
        let line = line.trim();
        if !line.starts_with('#') {
            continue;
        }
        let num = line.split_whitespace().next().unwrap_or("#?");
        // Function name sits between " in " and " (".
        let func = line
            .split_once(" in ")
            .map(|(_, rest)| rest)
            .and_then(|rest| rest.split_once(" (").map(|(f, _)| f))
            .unwrap_or("??")
            .trim();
        // Source location sits after " at ".
        let loc = line.split_once(" at ").map(|(_, l)| l.trim()).unwrap_or("??");
        frames.push(format!("{num} {func} at {loc}"));
    }
    frames.join("\n")
}

/// Helper: a `&Path` as `&str` for passing to `Command` args.
fn path(p: &std::path::Path) -> &str {
    p.to_str().unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_to_bytes_decodes_hex_and_skips_markers() {
        let block = "#CD:BEGIN#\n#CD:5a450200\n#CD:0300\n#CD:END#";
        assert_eq!(block_to_bytes(block).unwrap(), vec![0x5a, 0x45, 0x02, 0x00, 0x03, 0x00]);
        assert!(block_to_bytes("#CD:BEGIN#\n#CD:END#").is_err()); // no payload
        assert!(block_to_bytes("#CD:zz").is_err()); // non-hex payload
    }

    #[test]
    fn block_to_bytes_refuses_an_aborted_dump() {
        // The firmware dropped bytes mid-dump: decoding the rest would yield
        // a byte-shifted, plausible-but-wrong coredump.
        let block = "#CD:BEGIN#\n#CD:5a450200\n#CD:ERROR CANNOT DUMP#\n#CD:END#";
        let err = block_to_bytes(block).unwrap_err();
        assert!(err.contains("incomplete"), "got: {err}");
    }

    #[test]
    fn block_to_bytes_refuses_an_odd_length_line() {
        // A character lost on the UART: the only corruption the non-hex check
        // can't catch. Must error, not silently drop the trailing nibble.
        assert!(block_to_bytes("#CD:BEGIN#\n#CD:5a450\n#CD:END#").is_err());
    }

    #[test]
    fn dump_lines_are_recognized_for_log_filtering() {
        assert!(is_dump_line("[00:00:00] <err> coredump: #CD:BEGIN#"));
        assert!(is_dump_line("\x1b[1;31m<err> coredump: #CD:5a45\x1b[0m"));
        assert!(!is_dump_line("[00:00:00] <inf> app: sensor read ok"));
    }

    #[test]
    fn strips_ansi_color_codes() {
        let line = "\x1b[1;31m<err> coredump: #CD:BEGIN#\x1b[0m";
        assert_eq!(strip_ansi(line), "<err> coredump: #CD:BEGIN#");
    }

    #[test]
    fn collector_captures_a_full_block() {
        let mut c = CoredumpCollector::new();
        assert!(c.feed("[00:00:00] <err> coredump: #CD:BEGIN#").is_none());
        assert!(c.feed("[00:00:00] <err> coredump: #CD:5a450200").is_none());
        let block = c
            .feed("[00:00:00] <err> coredump: #CD:END#")
            .expect("block should be returned on END");
        assert_eq!(block, "#CD:BEGIN#\n#CD:5a450200\n#CD:END#");
    }

    #[test]
    fn collector_ignores_lines_outside_a_block() {
        let mut c = CoredumpCollector::new();
        assert!(c.feed("Booting Zephyr OS").is_none());
        assert!(c.feed("About to crash...").is_none());
    }

    #[test]
    fn collector_resets_on_a_new_begin() {
        let mut c = CoredumpCollector::new();
        c.feed("#CD:BEGIN#");
        c.feed("#CD:deadbeef"); // stale, interrupted dump
        // A fresh BEGIN must discard the half-captured data.
        c.feed("#CD:BEGIN#");
        c.feed("#CD:5a450200");
        let block = c.feed("#CD:END#").unwrap();
        assert_eq!(block, "#CD:BEGIN#\n#CD:5a450200\n#CD:END#");
    }

    #[test]
    fn formats_gdb_backtrace_into_clean_frames() {
        let gdb = "\
0x0800046a in i2c_read_reg (reg=0 '\\000') at /home/x/main.c:30
30\treturn *bad_ptr;
#0  0x0800046a in i2c_read_reg (reg=0 '\\000') at /home/x/main.c:30
#1  0x080044ea in read_sensor () at /home/x/main.c:36
#2  0x080004b4 in main () at /home/x/main.c:43";
        let expected = "\
#0 i2c_read_reg at /home/x/main.c:30
#1 read_sensor at /home/x/main.c:36
#2 main at /home/x/main.c:43";
        assert_eq!(format_backtrace(gdb), expected);
    }
}
