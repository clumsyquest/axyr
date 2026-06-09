# crash_demo

A minimal Zephyr application that deliberately triggers a CPU fault and exposes
the full crash state to the host. It is the on-device foundation for Axyr's
crash reporting: capture what the hardware was really doing at the moment it
died.

## What it does

1. A nested call chain `main() → read_sensor() → i2c_read_reg()` dereferences
   `0xBADCAFE0`, an address not mapped on the STM32F401. The Cortex-M4 raises a
   precise BusFault. The chain is deliberate: it makes the captured **call
   stack** show a real path to the fault, not just the crashing line.
2. The app overrides `k_sys_fatal_error_handler` (a *weak* kernel symbol).
   Zephyr calls our version with the fault `reason` and the exception stack
   frame (`esf`), and we emit a single structured line the host can parse:
   `AXYR_CRASH v=1 reason=… pc=… lr=… …`.
3. With `CONFIG_DEBUG_COREDUMP=y` (see `prj.conf`), Zephyr *also* dumps a full
   CPU + stack snapshot as a `#CD:BEGIN# … #CD:END#` block. That is what the host
   unwinds offline into the complete call stack.

All of this is emitted over **SEGGER RTT**, not the serial port: `prj.conf`
routes the console to RTT (`CONFIG_RTT_CONSOLE=y`, `CONFIG_UART_CONSOLE=n`) with
synchronous logging (`CONFIG_LOG_MODE_IMMEDIATE=y`) so the fault/coredump reach
RTT before the CPU halts. The host reads it non-intrusively over the debug probe.

## Build & flash

See `../../docs/flashing-nucleo-f401re-on-linux.md` for the toolchain setup.

```bash
cd ~/zephyrproject/zephyr
west build -p always -b nucleo_f401re ~/axyr/firmware/crash_demo
st-info --probe --connect-under-reset
west flash
```

## Expected output

Read over RTT (e.g. `cargo run --bin rtt_read`, or captured by `axyr-engine`).
Zephyr prints its own fault dump first (`BUS FAULT`, registers, `BFAR`), then
the coredump block, then our structured line:

```
[00:00:00.000,000] <err> os: ***** BUS FAULT *****
[00:00:00.000,000] <err> os:   Precise data bus error
[00:00:00.000,000] <err> os:   BFAR Address: 0xbadcafe0
...
[00:00:00.028,000] <err> coredump: #CD:BEGIN#
[00:00:00.033,000] <err> coredump: #CD:5a4502...
[00:00:00.151,000] <err> coredump: #CD:END#
AXYR_CRASH v=1 reason=25 pc=0x0800046a lr=0x080004b5 ...
```

- `reason` — fault type, decoded by Zephyr (`25` = precise data bus error)
- `pc` — the faulting instruction; the host resolves it to a source line
- `#CD:` block — the coredump the host turns into the full call stack

## Note: the coredump captures what the handler cannot

The Cortex-M fault status registers (`CFSR`, `BFAR`, `HFSR`) are **not** usable
from `k_sys_fatal_error_handler`: Zephyr's arch layer reads them to build its own
dump, then clears them **before** calling our handler. By the time it runs they
are already zeroed.

This is exactly why the coredump matters. Zephyr captures the snapshot — fault
address, registers, and stack — at the instant of the fault, before that state
evaporates. The host replays it offline with GDB (no board required) to recover
the precise faulting address and the full backtrace. See `../../engine/README.md`.
