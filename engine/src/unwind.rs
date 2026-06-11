//! Native crash unwinding — no GDB, no Python.
//!
//! The previous crash pipeline shelled out to Zephyr's `coredump_gdbserver.py`
//! and `arm-zephyr-eabi-gdb` to turn a coredump into a call stack. That works on
//! a dev box but makes the engine depend on a toolchain at runtime, which blocks
//! a self-contained cloud binary. This module replaces it in pure Rust:
//!
//!   1. [`parse_registers`] reads the Zephyr coredump header + arch block to
//!      recover the faulting CPU registers (PC/SP/RA + callee-saved). The header
//!      names the target architecture; ARM Cortex-M and RISC-V (32-bit) blocks
//!      are decoded, anything else is refused (never decoded with the wrong
//!      table).
//!   2. [`backtrace`] walks the stack using the ELF's DWARF call-frame info
//!      (`.debug_frame`, via `gimli`), reading stack words through a caller-
//!      supplied closure — backed by SWD reads of the halted core (the stack is
//!      readable post-fault) or by the coredump's own memory blocks offline.
//!   3. Each frame's PC is symbolized with `addr2line` (already used elsewhere).
//!
//! The coredump binary format is Zephyr's own (subsys/debug/coredump): a
//! `<ccHHBBI>` header (`ZE`, version, target, ptr-size, flags, reason) followed
//! by typed blocks — `'A'` arch registers, `'M'` memory regions, `'T'` thread
//! metadata. Register orders match Zephyr's gdbstub parsers (`arm_cortex_m.py`,
//! `risc_v.py`).

use addr2line::Loader;
use gimli::{
    BaseAddresses, CfaRule, DebugFrame, LittleEndian, RegisterRule, UnwindContext, UnwindSection,
};
use object::{Object, ObjectSection};

/// The coredump's target architecture (header target code), driving the
/// register conventions used during unwinding.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Arch {
    /// Zephyr target code 3. DWARF regs r0..r15; sp = 13, pc = 15; return
    /// addresses carry the Thumb bit.
    ArmCortexM,
    /// Zephyr target code 4 (32-bit). DWARF regs x0..x31; sp = x2 (2),
    /// ra = x1 (1); the PC is not a DWARF register.
    RiscV,
}

impl Arch {
    /// The DWARF register number that holds the stack pointer.
    fn sp_dwarf(self) -> usize {
        match self {
            Arch::ArmCortexM => 13,
            Arch::RiscV => 2,
        }
    }
    /// Mask applied to recovered return addresses (ARM: clear the Thumb bit).
    fn ret_mask(self) -> u32 {
        match self {
            Arch::ArmCortexM => !1,
            Arch::RiscV => !0,
        }
    }
}

/// Recovered CPU registers, indexed by DWARF register number (ARM: r0..r15,
/// RISC-V: x0..x31). `None` = not in the dump. The PC is tracked separately —
/// on RISC-V it is not a DWARF register.
#[derive(Debug)]
pub struct Registers {
    pub arch: Arch,
    pub regs: [Option<u32>; 32],
    pub pc: Option<u32>,
    /// The Zephyr fatal-error reason code from the coredump header.
    pub reason: u32,
}

/// One resolved stack frame.
pub struct Frame {
    pub pc: u32,
    pub func: String,
    pub file: String,
    pub line: u32,
}

/// Parse the Zephyr coredump header and arch block into CPU registers.
pub fn parse_registers(dump: &[u8]) -> Result<Registers, String> {
    if dump.len() < 12 || &dump[0..2] != b"ZE" {
        return Err("not a Zephyr coredump (bad magic)".to_string());
    }
    let rd32 = |o: usize| u32::from_le_bytes([dump[o], dump[o + 1], dump[o + 2], dump[o + 3]]);
    let rd16 = |o: usize| u16::from_le_bytes([dump[o], dump[o + 1]]);
    // The header's target code says which architecture's register layout the
    // arch block uses (coredump.h). Decoding an unknown one with a known
    // table would fabricate a plausible-but-wrong backtrace — refuse instead.
    let arch = match rd16(4) {
        3 => Arch::ArmCortexM,
        4 => Arch::RiscV,
        other => {
            return Err(format!(
                "coredump target code {other} — unwinding not implemented for this architecture yet"
            ));
        }
    };
    let reason = rd32(8);

    let mut regs = [None; 32];
    let mut pc = None;
    let mut off = 12; // past the fixed header
    while off < dump.len() {
        match dump[off] {
            b'A' => {
                // <cHH>: id, hdr_version, num_bytes
                if off + 5 > dump.len() {
                    break;
                }
                let ver = rd16(off + 1);
                let num = rd16(off + 3) as usize;
                let data = off + 5;
                if data + num > dump.len() {
                    return Err("coredump arch block truncated".to_string());
                }
                match arch {
                    Arch::ArmCortexM => parse_arm_arch(&dump[data..data + num], ver, &mut regs, &mut pc)?,
                    Arch::RiscV => parse_riscv_arch(&dump[data..data + num], ver, &mut regs, &mut pc)?,
                }
                off = data + num;
            }
            b'M' => {
                // <cH> + <II> (start,end) + (end-start) bytes — skipped here;
                // stack memory is read live over SWD during unwinding.
                if off + 11 > dump.len() {
                    break;
                }
                let start = rd32(off + 3);
                let end = rd32(off + 7);
                off = off + 11 + end.saturating_sub(start) as usize;
            }
            b'T' => {
                // <cHH> + num_bytes
                if off + 5 > dump.len() {
                    break;
                }
                off = off + 5 + rd16(off + 3) as usize;
            }
            _ => break,
        }
    }

    if pc.is_none() {
        return Err("coredump has no PC (no arch block?)".to_string());
    }
    Ok(Registers { arch, regs, pc, reason })
}

/// Decode the ARM Cortex-M arch register block. Order (matching Zephyr's
/// `arm_cortex_m.py`): r0,r1,r2,r3,r12,lr,pc,xpsr,sp, then (v2+) r4..r11, then
/// (v3+) callee-saved valid+offset (ignored — r4..r11 are inline).
fn parse_arm_arch(
    data: &[u8],
    ver: u16,
    regs: &mut [Option<u32>; 32],
    pc: &mut Option<u32>,
) -> Result<(), String> {
    let min_words = if ver <= 1 { 9 } else { 17 };
    if data.len() < min_words * 4 {
        return Err(format!("ARM arch block too short for v{ver}"));
    }
    let w = |i: usize| u32::from_le_bytes([data[i * 4], data[i * 4 + 1], data[i * 4 + 2], data[i * 4 + 3]]);
    regs[0] = Some(w(0));
    regs[1] = Some(w(1));
    regs[2] = Some(w(2));
    regs[3] = Some(w(3));
    regs[12] = Some(w(4));
    regs[14] = Some(w(5)); // lr
    regs[15] = Some(w(6)); // pc (DWARF reg 15 on ARM)
    *pc = Some(w(6));
    // w(7) = xpsr (not tracked)
    regs[13] = Some(w(8)); // sp
    if ver > 1 {
        for (slot, idx) in (4..=11).zip(9..=16) {
            regs[slot] = Some(w(idx));
        }
    }
    Ok(())
}

/// Decode the RISC-V arch register block. Order (matching Zephyr's
/// `risc_v.py` / arch/riscv/core/coredump.c): x0..x31 then pc — GDB order,
/// which is also the DWARF numbering. v3 = 32-bit; v4 (RV64) is refused (the
/// engine unwinds 32-bit targets).
fn parse_riscv_arch(
    data: &[u8],
    ver: u16,
    regs: &mut [Option<u32>; 32],
    pc: &mut Option<u32>,
) -> Result<(), String> {
    if ver != 3 {
        return Err(format!("RISC-V arch block v{ver} not supported (only v3 / 32-bit)"));
    }
    if data.len() < 33 * 4 {
        return Err("RISC-V arch block too short (expected 33 words)".to_string());
    }
    let w = |i: usize| u32::from_le_bytes([data[i * 4], data[i * 4 + 1], data[i * 4 + 2], data[i * 4 + 3]]);
    for (i, reg) in regs.iter_mut().enumerate() {
        *reg = Some(w(i));
    }
    *pc = Some(w(32));
    Ok(())
}

/// Unwind the call stack from the faulting registers, reading stack memory via
/// `read_word` (returns the 32-bit word at an address, or `None` if unreadable).
/// Frames are symbolized with `addr2line`. Stops at the first unrecoverable
/// frame, a null return address, a repeated PC, or `max_frames`.
pub fn backtrace(
    elf: &str,
    start: &Registers,
    mut read_word: impl FnMut(u32) -> Option<u32>,
    max_frames: usize,
) -> Result<Vec<Frame>, String> {
    let data = std::fs::read(elf).map_err(|e| format!("read elf {elf}: {e}"))?;
    let file = object::File::parse(&*data).map_err(|e| format!("parse elf: {e}"))?;
    let section = file
        .section_by_name(".debug_frame")
        .ok_or("ELF has no .debug_frame (DWARF CFI) — cannot unwind natively")?;
    let frame_data = section
        .uncompressed_data()
        .map_err(|e| format!("read .debug_frame: {e}"))?;

    let mut debug_frame = DebugFrame::new(&frame_data, LittleEndian);
    debug_frame.set_address_size(4);
    let bases = BaseAddresses::default();
    let mut ctx = UnwindContext::new();

    let loader = Loader::new(elf).map_err(|e| format!("symbolizer: {e}"))?;

    let arch = start.arch;
    let sp = arch.sp_dwarf();
    let mut regs = start.regs;
    let mut pc_reg = start.pc;
    let mut frames = Vec::new();
    let mut seen = std::collections::HashSet::new();

    'unwind: for depth in 0..max_frames {
        let Some(pc) = pc_reg else { break };
        // For a return address (caller frames), step back one byte so the source
        // line is the call site, not the instruction after it.
        let sym_addr = if depth == 0 { pc } else { pc.wrapping_sub(1) };
        // Expand inlined functions at this PC (innermost first), like gdb does.
        for frame in frames_at(&loader, pc, sym_addr) {
            let is_main = frame.func == "main";
            frames.push(frame);
            if is_main {
                break 'unwind; // stop at main, matching gdb's convention
            }
        }

        if !seen.insert(pc) {
            break; // loop guard
        }

        let fde = match debug_frame.fde_for_address(&bases, pc as u64, DebugFrame::cie_from_offset) {
            Ok(f) => f,
            Err(_) => break, // no CFI for this PC — end of the recoverable stack
        };
        let ra_reg = fde.cie().return_address_register().0 as usize;
        let row = match fde.unwind_info_for_address(&debug_frame, &bases, &mut ctx, pc as u64) {
            Ok(r) => r,
            Err(_) => break,
        };

        // Canonical Frame Address for this frame.
        let cfa = match row.cfa() {
            CfaRule::RegisterAndOffset { register, offset } => {
                match regs[register.0 as usize] {
                    Some(base) => (base as i64 + offset) as u32,
                    None => break,
                }
            }
            _ => break, // expression-based CFA: not supported (rare on Cortex-M)
        };

        // Recover the caller's registers per the CFI rules for this row.
        let mut next = regs;
        for i in 0..32 {
            next[i] = match row.register(gimli::Register(i as u16)) {
                Some(RegisterRule::Undefined) => None,
                Some(RegisterRule::SameValue) => regs[i],
                Some(RegisterRule::Offset(o)) => read_word((cfa as i64 + o) as u32),
                Some(RegisterRule::ValOffset(o)) => Some((cfa as i64 + o) as u32),
                Some(RegisterRule::Register(r)) => regs[r.0 as usize],
                Some(RegisterRule::Constant(c)) => Some(c as u32),
                Some(_) => None,    // expression / architectural: unsupported
                None => regs[i],    // no rule: register is preserved (same value)
            };
        }

        let Some(ra) = next.get(ra_reg).copied().flatten() else { break };
        if ra == 0 {
            break;
        }
        next[sp] = Some(cfa);
        pc_reg = Some(ra & arch.ret_mask()); // ARM: clear the Thumb bit
        if arch == Arch::ArmCortexM {
            next[15] = pc_reg; // on ARM the PC is also DWARF reg 15
        }
        regs = next;
    }

    Ok(frames)
}

/// All logical frames at an address: the inlined-function chain (innermost
/// first) via DWARF, falling back to the symbol table if there's no line info.
fn frames_at(loader: &Loader, pc: u32, sym_addr: u32) -> Vec<Frame> {
    let mut out = Vec::new();
    if let Ok(mut iter) = loader.find_frames(sym_addr as u64) {
        while let Ok(Some(frame)) = iter.next() {
            let func = frame
                .function
                .as_ref()
                .and_then(|f| f.demangle().ok().map(|c| c.into_owned()))
                .unwrap_or_else(|| "??".to_string());
            let (file, line) = frame
                .location
                .as_ref()
                .map(|l| (l.file.unwrap_or("?").to_string(), l.line.unwrap_or(0)))
                .unwrap_or_else(|| ("?".to_string(), 0));
            out.push(Frame { pc, func, file, line });
        }
    }
    if out.is_empty() {
        // No DWARF line info here: fall back to the ELF symbol table.
        let func = loader.find_symbol(sym_addr as u64).unwrap_or("??").to_string();
        out.push(Frame { pc, func, file: "?".to_string(), line: 0 });
    }
    out
}

/// Format frames as a compact call stack: `#0 func at file:line`.
pub fn format_backtrace(frames: &[Frame]) -> String {
    frames
        .iter()
        .enumerate()
        .map(|(i, f)| format!("#{i} {} at {}:{}", f.func, f.file, f.line))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real `crash_demo` coredump captured from the Nucleo-F401RE (precise bus
    /// fault reading 0xBADCAFE0): ZE header + ARM v3 arch block + a memory block.
    const DUMP_HEX: &str = "5a45020003000500190000004103004c005d110008440b00200000000000a0dcbaad1f0008b50400086a04000800000001b81d0020000000000000000000000000000000000000000000000000000000000000000000000000000000004d0100e8060020a807002058050020c807002000000000010000008000000000000000";

    fn decode(hex: &str) -> Vec<u8> {
        let clean: String = hex.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        (0..clean.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&clean[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn parses_arm_registers_from_a_real_coredump() {
        let regs = parse_registers(&decode(DUMP_HEX)).unwrap();
        assert_eq!(regs.arch, Arch::ArmCortexM);
        assert_eq!(regs.reason, 25); // precise bus fault
        assert_eq!(regs.pc, Some(0x0800_046a));
        assert_eq!(regs.regs[14], Some(0x0800_04b5)); // lr
        assert_eq!(regs.regs[13], Some(0x2000_1db8)); // sp
        assert_eq!(regs.regs[3], Some(0xbadc_a000)); // the bad pointer base
    }

    /// A synthetic RISC-V coredump: ZE header (tgt 4) + v3 arch block with
    /// x0..x31 then pc, per Zephyr's arch/riscv/core/coredump.c.
    fn riscv_dump() -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(b"ZE"); // magic
        d.extend_from_slice(&2u16.to_le_bytes()); // hdr version
        d.extend_from_slice(&4u16.to_le_bytes()); // target: RISC-V
        d.push(5); // ptr size (log2 bits, as emitted)
        d.push(0); // flags
        d.extend_from_slice(&2u32.to_le_bytes()); // reason: stack overflow
        d.push(b'A');
        d.extend_from_slice(&3u16.to_le_bytes()); // arch block v3 (32-bit)
        d.extend_from_slice(&(33u16 * 4).to_le_bytes()); // num bytes
        for i in 0..32u32 {
            d.extend_from_slice(&(0x1000_0000 + i).to_le_bytes()); // x0..x31
        }
        d.extend_from_slice(&0x4038_0abcu32.to_le_bytes()); // pc
        d
    }

    #[test]
    fn parses_riscv_registers() {
        let regs = parse_registers(&riscv_dump()).unwrap();
        assert_eq!(regs.arch, Arch::RiscV);
        assert_eq!(regs.reason, 2);
        assert_eq!(regs.pc, Some(0x4038_0abc));
        assert_eq!(regs.regs[1], Some(0x1000_0001)); // ra = x1
        assert_eq!(regs.regs[2], Some(0x1000_0002)); // sp = x2
        assert_eq!(regs.regs[31], Some(0x1000_001f)); // t6 = x31
    }

    #[test]
    fn refuses_unknown_target_instead_of_guessing() {
        // Same header but target code 5 (Xtensa): must refuse, never decode
        // with another architecture's table.
        let mut d = riscv_dump();
        d[4] = 5;
        let err = parse_registers(&d).unwrap_err();
        assert!(err.contains("not implemented"), "{err}");
    }

    #[test]
    fn rejects_non_coredump() {
        assert!(parse_registers(b"not a dump").is_err());
    }
}
