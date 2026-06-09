# Architecture

> Living design note. Reflects decisions made so far; sections marked _(open)_
> are still being defined.

## Goal

Give a **living map of the embedded system** — to a human and to an AI agent.
Not just "it crashed", but: what components exist (sensors, actuators, buses),
what runs and when, the data flowing through the peripherals, the registers, and
crashes. Like a developer who owns the physical board, but with the internal,
real-time truth they can't see — and rendered in plain language.

The crash explainer was the first capability (it proved the thesis); the product
is the broader system representation.

## Layers

```
┌ on-device ─────────────────────────────────────────────────────────────┐
│ Firmware (Zephyr, C)                                                     │
│   • custom k_sys_fatal_error_handler → structured AXYR_CRASH line        │
│   • CONFIG_DEBUG_COREDUMP → full CPU/stack snapshot (#CD:… block)        │
└──────────────┬──────────────────────────────────┬───────────────────────┘
        serial (VCP)                         debug probe (SWD)
               │                                   │
┌ local ───────▼───────────────────────────────────▼──────────────────────┐
│ Local agent (thin, open-source — NOT the product, just plumbing)         │
│   captures serial + drives the SWD probe; forwards data / relays actions │
└──────────────────────────────┬───────────────────────────────────────────┘
                               │  (today: in-process; later: network)
┌ engine (the product) ────────▼───────────────────────────────────────────┐
│ Analyzer (Rust)                                                          │
│   parse crash · symbolize (addr2line) · coredump→backtrace (GDB) ·       │
│   recent serial log · live memory/register reads (probe-rs) ·            │
│   system map (devicetree) · register decode (SVD) · variables (DWARF)    │
│                                                                          │
│   ├─ MCP server  → AI agents: get_last_crash + reboot/flash/read_memory  │
│   └─ data API    → the dashboard                                         │
└───────────────────────────────────────────────────────────────────────────┘
                               │
                     ┌─────────▼──────────┐
                     │ Dashboard (web UI) │  built/designed by the project owner
                     │ human view, plain  │  (not part of this engine work)
                     └────────────────────┘
```

## Key decision: OS-agnostic core + backends

The deep truth (fault registers, memory, live state via SWD) lives at the
**architecture/hardware** level, not the OS level — so it is not locked to
Zephyr. The design is an **OS-agnostic core** with pluggable **backends**:

- **Zephyr** — the first backend (structured, large community, devicetree).
- **Debug probe (SWD/JTAG)** — the universal backend: it reads the chip directly,
  so it works on bare-metal and any RTOS. The path to OS independence.
- Future: FreeRTOS, ThreadX, …

Already true today: STM32 = ARM Cortex-M, ESP32 = Xtensa/RISC-V → two
architectures → a core plus per-target backends from day one.

## Where the data comes from

| Source | What it gives | Cost |
|---|---|---|
| **Devicetree** (`build/zephyr/zephyr.dts`) | Static system map — every peripheral/sensor/actuator + address. Zephyr generates it; we parse it. | cheap |
| **SVD** (CMSIS) | Decode raw peripheral registers into named fields → register state in plain language. | cheap |
| **ELF / DWARF** | Every global variable's address + type → live "watch". Symbol resolution for the PC. | cheap |
| **SWD live reads** (probe-rs) | Read memory/registers **while the core runs**, low overhead. The moat. | medium |
| **Coredump + GDB** | Crash post-mortem: full call stack, registers, faulting address. | done |

## Deployment & business _(open)_

- The **product runs server-side** (e.g. Railway/Render); the local agent stays
  thin and carries no IP. Reason: protect the engine, manage users.
- **Build in public, open-source first**; reach users via HN, X, Hugging Face.
- **Commercial later** — billing + user management will come, no fixed date; the
  early phase is open access for adoption. Not now.
- The **dashboard is owned by the project owner** (design + build). The engine's
  job is to expose a clean data contract it can consume.

## Roadmap (marches)

1. ✅ Crash explainer — cause, location, call stack, recent log; over MCP.
2. ✅ Debug-probe foundation (probe-rs) — attach, live read, reset, flash.
3. ✅ Agent actions over MCP — reboot, flash, read_memory.
4. ⬜ Live system state — the system map (devicetree), register decode (SVD),
   live variables (DWARF). Needs a looping demo firmware to show live reads.
5. ⬜ ESP32 backend; later FreeRTOS / bare-metal.
6. 🔒 Web dashboard — owned by the project owner.
