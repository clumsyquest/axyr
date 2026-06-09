# axyr — Zephyr module

Drop-in on-device support for Axyr. A firmware enables everything Axyr needs
with **one line** instead of hand-editing `prj.conf`:

```
CONFIG_AXYR=y
```

It turns on (and compiles the trace ring):
- **SEGGER RTT** console — telemetry/crash over RTT, no serial port.
- **in-memory coredump** — fast, host-read over SWD.
- **thread analyzer** (auto) — per-thread stack + CPU.
- **tracing (user hooks)** + the context-switch **trace ring** (`axyr_trace`).
- synchronous logging so fault/coredump reach RTT before halt.

## Use it

Point the build at this module and enable the switch:

```bash
west build -b <board> <app> -- -DZEPHYR_EXTRA_MODULES=/path/to/axyr/firmware/axyr
# with CONFIG_AXYR=y in the app's prj.conf
```

(Or add `axyr` to your west manifest / `ZEPHYR_EXTRA_MODULES`.)

The host engine then reads it all over the probe — see `../../engine`.
