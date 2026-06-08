# crash_demo

A minimal Zephyr application that deliberately triggers a CPU fault and then
**intercepts it with a custom fatal error handler**. It is the on-device
foundation for Axyr's crash reporting: capture the CPU state at the exact moment
of a crash.

## What it does

1. `main()` reads from `0xBADCAFE0`, an address not mapped on the STM32F401.
   The Cortex-M4 raises a precise BusFault.
2. Instead of leaving the crash to Zephyr alone, the app overrides
   `k_sys_fatal_error_handler` (a *weak* kernel symbol). Zephyr calls our version
   and passes it the fault `reason` plus the exception stack frame (`esf`) — a
   snapshot of the CPU registers at the moment of the fault.
3. Our handler prints that captured state, then halts the CPU.

## Build & flash

See `../../docs/flashing-nucleo-f401re-on-linux.md` for the toolchain setup.

````bash
cd ~/zephyrproject/zephyr
west build -p always -b nucleo_f401re ~/axyr/firmware/crash_demo
st-info --probe --connect-under-reset
west flash
screen /dev/ttyACM0 115200
````

## Expected output

Zephyr prints its own fault dump first (`BUS FAULT`, registers, `BFAR`), then our
handler prints its captured block:

````
>>> AXYR caught a crash! (reason=25)
    pc   = 0x08000508
    lr   = 0x08000505
    xpsr = 0x61000000
    r0   = 0x0800399f
    ...
>>> halting
````

- `reason` — fault type, already decoded by Zephyr (`25` = precise data bus error)
- `pc` — the faulting instruction; later resolved to the exact source line (addr2line)
- `r0`–`r3`, `lr`, `xpsr` — CPU register snapshot at the crash

## Note: fault status registers are ephemeral

The Cortex-M fault status registers (`CFSR`, `BFAR`, `HFSR`) are **not** usable
from our handler. Zephyr's arch layer reads them to build its own dump, then
clears them **before** calling `k_sys_fatal_error_handler`. By the time our
handler runs they are already zeroed. The reliable data is what Zephyr hands us:
`reason` and `esf`. Capturing the raw `BFAR` would require hooking earlier in the
fault path (future work).
````
````
