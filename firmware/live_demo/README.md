# live_demo

A minimal "moving target" so the host can demonstrate reading **live** system
state over SWD while the core runs.

It just increments a global counter every 250 ms and prints it:

```c
volatile uint32_t axyr_counter = 0;
while (1) { axyr_counter++; printk("counter=%u\n", axyr_counter); k_msleep(250); }
```

There is nothing Axyr-specific on the device. The host reads `axyr_counter` by
resolving its address from the ELF symbol table and sampling that address live —
exactly what it does for any real application's globals. This app only makes the
capability visible; a real project is observed the same way.

## Build & flash

```bash
cd ~/zephyrproject/zephyr
west build -p always -b nucleo_f401re ~/axyr/firmware/live_demo
west flash
```

## Reading it from the host

```bash
export AXYR_ELF=~/zephyrproject/zephyr/build/zephyr/zephyr.elf
# via the MCP server's read_variable tool, e.g.:
#   read_variable { "name": "axyr_counter" }
```

## Note: reading live state is not free on this probe

On the Nucleo's ST-LINK, SRAM is only accessible to the debugger while the core
is **halted** — a running/sleeping core returns zeros for AHB memory. So the host
samples a variable with a brief **halt → read → resume** (sub-millisecond). Truly
zero-overhead background reads would need a different probe (J-Link) or an RTT
channel. This is the real "capture without overhead" challenge; the brief-halt
snapshot is the pragmatic path. See `../../engine/README.md`.
