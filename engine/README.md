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
- `src/main.rs` — the serial listener that wires it together and writes the
  "last crash" file.
- `src/bin/mcp.rs` — a small MCP server exposing `get_last_crash`, which simply
  returns the contents of that file.

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
