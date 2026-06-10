//! Marche 1 smoke test for the debug-probe foundation.
//!
//! Validates, against a real board, that we can attach over SWD, read live
//! memory, report core status, reset, and (optionally) flash an ELF.
//!
//!   probe_check [chip] [elf-to-flash]
//!
//! Without a chip argument it auto-detects the target (the same layered
//! detection `axyr` uses). If an ELF path is given, it is flashed after the
//! read checks.

use std::path::PathBuf;

use axyr_engine::probe::Probe;

fn main() {
    let chip = std::env::args().nth(1);
    let elf = std::env::args().nth(2).map(PathBuf::from);

    let mut probe = match chip {
        Some(chip) => {
            println!("attaching to {chip} over SWD ...");
            match Probe::attach(&chip) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("attach failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        None => {
            println!("auto-detecting the target over SWD ...");
            match Probe::attach_auto(None) {
                Ok((p, d)) => {
                    println!("detected {} — {}", d.chip, d.method);
                    p
                }
                Err(e) => {
                    eprintln!("auto-attach failed: {e}");
                    std::process::exit(1);
                }
            }
        }
    };
    println!("attached.");

    match probe.status() {
        Ok(s) => println!("core status: {s}"),
        Err(e) => eprintln!("status error: {e}"),
    }

    // SCB CPUID (0xE000ED00) is a fixed ROM value — a safe proof that live
    // memory reads over SWD work. On a Cortex-M4 it reads 0x410FC24x.
    match probe.read_word(0xE000_ED00) {
        Ok(v) => println!("SCB CPUID  @0xE000ED00 = {v:#010x}"),
        Err(e) => eprintln!("read CPUID error: {e}"),
    }
    // First words of flash (the initial stack pointer and reset vector).
    let mut vt = [0u32; 2];
    match probe.read_words(0x0800_0000, &mut vt) {
        Ok(()) => println!("vector tbl @0x08000000 = SP {:#010x}  reset {:#010x}", vt[0], vt[1]),
        Err(e) => eprintln!("read vector table error: {e}"),
    }

    println!("resetting target ...");
    match probe.reset() {
        Ok(()) => println!("reset ok"),
        Err(e) => eprintln!("reset error: {e}"),
    }

    if let Some(path) = elf {
        println!("flashing {} ...", path.display());
        match probe.flash_elf(&path) {
            Ok(()) => println!("flash ok"),
            Err(e) => eprintln!("flash error: {e}"),
        }
    }
}
