# rtt_demo

A minimal firmware that emits telemetry over **SEGGER RTT** instead of UART, to
prove the host can read it **non-intrusively** — no CPU halt, and it survives the
core sleeping (WFI) between writes.

```c
while (1) { printk("counter=%u\n", counter++); k_msleep(250); }
```

`prj.conf` routes the console to RTT:

```
CONFIG_USE_SEGGER_RTT=y
CONFIG_RTT_CONSOLE=y
CONFIG_CONSOLE=y
CONFIG_UART_CONSOLE=n
```

So `printk()` writes to a RAM ring buffer (a few cycles, no blocking, no halt).
The host drains that buffer over the debug probe.

## Why RTT (vs the serial channel)

The UART serial channel was only used to prove the thesis. RTT is the real
transport: it doesn't occupy a serial port, it's non-intrusive, and it's
deterministic even while the core sleeps — the data is buffered, so a read that
misses a sleeping window just retries and loses nothing (polling a variable over
SWD would instead read a false zero during sleep). The engine reads it the same
way regardless of where it runs (local agent forwarding to the cloud later).

## Build, flash & read

```bash
cd ~/zephyrproject/zephyr
west build -p always -b nucleo_f401re ~/axyr/firmware/rtt_demo
west flash
# then, from the engine:
cargo run --bin rtt_read         # streams the RTT telemetry, no halt
```
