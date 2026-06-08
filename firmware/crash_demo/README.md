# crash_demo

A minimal Zephyr application that deliberately triggers a CPU fault, used to
study the raw fault output Zephyr produces — the starting point for Axyr's
on-device crash reporting.

## What it does

`main()` reads from `0xBADCAFE0`, an address not mapped on the STM32F401. The
Cortex-M4 raises a precise BusFault, which Zephyr's fatal error handler catches
and dumps over the serial console.

## Build & flash

```bash
cd ~/zephyrproject/zephyr
west build -p always -b nucleo_f401re ~/axyr/firmware/crash_demo
st-info --probe --connect-under-reset
west flash
screen /dev/ttyACM0 115200
```

## Expected output

A fault dump containing:
- the fault type (`BUS FAULT`)
- `BFAR` = `0xbadcafe0` — the address that faulted
- `pc` — the faulting instruction, which maps back to the read in `main.c`
- a snapshot of the CPU registers

The `pc` is the key value: it will later be resolved to the exact source line.
