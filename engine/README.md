# axyr-engine

The host-side engine: it listens to the board over serial, turns a crash into a
human-readable post-mortem (cause, source location, **full call stack**), and
exposes it to AI agents over MCP.

## Layout

- `src/lib.rs` — crash logic: parse the `AXYR_CRASH` line, decode the fatal
  reason, symbolize the PC via `addr2line`, format the report. Pure and unit
  tested (no hardware needed: `cargo test`).
- `src/coredump.rs` — capture the Zephyr coredump block from the serial stream
  and resolve it into a call stack by driving Zephyr's tooling + GDB offline.
- `src/recent_log.rs` — a ring buffer of recent serial output, attached to the
  report as "what was happening just before the crash".
- `src/probe.rs` — debug-probe (SWD) access via `probe-rs`: attach to a running
  core, read live memory/registers, reset, and flash. The foundation for live
  system state and for agent actions (flash/reboot/debug).
- `src/system_map.rs` — parses the Zephyr devicetree (`zephyr.dts`) into the
  board's hardware map: peripherals, sensors/actuators, addresses, on/off state.
- `src/symbols.rs` — resolves a global variable to its address/size from the
  firmware ELF symbol table (generic: any ELF, any variable).
- `src/bin/probe_check.rs` — hardware smoke test for the probe foundation:
  `probe_check [chip] [elf-to-flash]` (defaults to the Nucleo-F401RE).
- `src/main.rs` — the serial listener that wires it together and writes the
  "last crash" file.
- `src/bin/mcp.rs` — an MCP server (stdio) exposing the system to an AI agent:
  `get_last_crash` and `get_system_map` (reads), plus `reboot_board`,
  `flash_firmware`, `read_memory`, and `read_variable` (resolve a global from the
  ELF and read it live). Probe actions attach per-call; set `AXYR_CHIP` for
  another chip, `AXYR_DTS` for `get_system_map`, and `AXYR_ELF` for
  `read_variable`.

### Reading live state (the overhead caveat)

On the Nucleo's ST-LINK, SRAM is only debugger-visible while the core is
**halted** — a running/sleeping core returns zeros for AHB memory (the CPU's own
registers, e.g. CPUID, stay readable). So `read_variable` takes a snapshot with a
brief **halt → read → resume** (sub-millisecond). Truly zero-overhead background
reads need a different probe (J-Link) or an RTT channel. This is the real
"capture without overhead" challenge.

## Build & test

```bash
cargo build
cargo test
```

## Run

```bash
axyr-engine <serial-port> <elf> <addr2line> <crash-file>
```

Example:

```bash
axyr-engine /dev/ttyACM0 \
  ~/zephyrproject/zephyr/build/zephyr/zephyr.elf \
  ~/zephyr-sdk-1.0.1/gnu/arm-zephyr-eabi/bin/arm-zephyr-eabi-addr2line \
  /tmp/last_crash.txt
```

This always produces the fast report (cause + faulting location). To also
resolve the **full call stack** from the coredump, point the engine at the
Zephyr coredump tooling via environment variables:

```bash
export AXYR_GDB=~/zephyr-sdk-1.0.1/gnu/arm-zephyr-eabi/bin/arm-zephyr-eabi-gdb
export AXYR_COREDUMP_LOG_PARSER=~/zephyrproject/zephyr/scripts/coredump/coredump_serial_log_parser.py
export AXYR_COREDUMP_GDBSERVER=~/zephyrproject/zephyr/scripts/coredump/coredump_gdbserver.py
# AXYR_PYTHON is optional (defaults to "python3")
```

If those are unset, the engine still runs — it just omits the call stack.

### Coredump pipeline

When the firmware is built with `CONFIG_DEBUG_COREDUMP=y`, a crash emits a
`#CD:BEGIN# … #CD:END#` block over serial. The engine:

1. collects that block (`CoredumpCollector`, stripping the log/ANSI noise),
2. runs Zephyr's `coredump_serial_log_parser.py` to get a binary coredump,
3. drives GDB offline — the `coredump_gdbserver.py --pipe` script acts as a GDB
   remote target — and keeps the `bt` frames.

The resolved call stack is appended to the crash report, so `get_last_crash`
returns it to the agent too.

Requires `pyelftools` for the coredump scripts: `pip install pyelftools`.

## Connecting to the board on Linux

The Nucleo's ST-LINK exposes both the debug interface (SWD) and the virtual COM
port (`/dev/ttyACM0`) on one composite USB device, which makes the serial port
flaky after repeated resets. Two things that help:

- Flash / probe **under reset**: `st-flash --connect-under-reset ...` /
  `st-info --probe --connect-under-reset`.
- Trigger a clean re-run with `openocd -f interface/stlink.cfg -f
  target/stm32f4x.cfg -c "init; reset run; exit"` (a plain `st-flash reset` can
  leave the core halted).
- If the VCP wedges, USB-reset the ST-LINK (an `USBDEVFS_RESET` ioctl on the
  device under `/dev/bus/usb/...`) to make Linux re-enumerate it.
