//! Dev tool: validate the native unwinder against a real board.
//!
//! Parses CPU registers from a captured Zephyr coredump, then unwinds the stack
//! by reading stack memory live over SWD from the (halted/looping) target — the
//! same path the agent uses post-fault. Prints the call stack to compare against
//! gdb. Usage: `unwind_check <elf> <coredump.bin>`.

use std::env;

use axyr_engine::probe::Probe;
use axyr_engine::unwind;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: {} <elf> <coredump.bin>", args[0]);
        std::process::exit(1);
    }
    let dump = std::fs::read(&args[2]).expect("read coredump");
    let regs = unwind::parse_registers(&dump).expect("parse registers");
    eprintln!(
        "registers: pc={:#010x} sp={:#010x} lr={:#010x} reason={}",
        regs.regs[15].unwrap_or(0),
        regs.regs[13].unwrap_or(0),
        regs.regs[14].unwrap_or(0),
        regs.reason
    );

    let mut probe = match env::var("AXYR_CHIP") {
        Ok(chip) => Probe::attach(&chip),
        Err(_) => Probe::attach_auto(None).map(|(p, _)| p),
    }
    .expect("attach probe");
    let frames = unwind::backtrace(&args[1], &regs, |addr| probe.read_word(addr as u64).ok(), 32)
        .expect("unwind");
    println!("{}", unwind::format_backtrace(&frames));
}
