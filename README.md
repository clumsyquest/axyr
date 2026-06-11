# Axyr

The "DevTools / F12 for microcontrollers": an open-source layer that captures a microcontroller's real internal state in real time and makes it readable — for developers and for AI agents (via MCP).

## Why
Debugging embedded systems often means flying blind: a board reboots, a HardFault fires, and you're left guessing. Axyr turns that opaque failure into a clear, structured explanation — cause, function, file:line, registers — exposed both to humans and to AI agents.

## Install

Linux, macOS, **and Git Bash on Windows**:

```bash
curl -fsSL https://raw.githubusercontent.com/clumsyquest/axyr/main/install.sh | sh
```

Windows (native PowerShell):

```powershell
irm https://raw.githubusercontent.com/clumsyquest/axyr/main/install.ps1 | iex
```

Linux (x86_64 / aarch64), macOS (Intel / Apple Silicon) and Windows (x86_64).
The script downloads the latest
[release](https://github.com/clumsyquest/axyr/releases), verifies its checksum,
and installs `axyr` to `~/.local/bin` (Windows:
`%LOCALAPPDATA%\Programs\axyr`) — no sudo, no admin. (On Windows the board's
ST-LINK needs the usual ST USB driver.) Then, from your Zephyr project, with
the board plugged in:

```bash
axyr
```

It finds your firmware build, auto-detects the chip, and serves the dashboard
API locally. (Pin a version with `AXYR_VERSION=v0.1.0`, change the directory
with `AXYR_INSTALL_DIR=...`.)

## Structure
- `firmware/`  — on-device code (C, Zephyr) running on the chip
- `engine/`    — host engine + MCP server (Rust)
- `dashboard/` — web interface (TypeScript)
- `docs/`      — design notes and documentation (see [`docs/architecture.md`](docs/architecture.md) and the [data contract](docs/data-contract.md))

## What works today
- **Crash post-mortem** — on a fault, a clear report: cause in plain words, function + `file:line`, the full **call stack** (Zephyr coredump replayed through GDB), and the **recent serial output** leading up to it.
- **Debug-probe access (SWD)** — attach to a *running* core and read live memory/registers, reset, and flash, via [`probe-rs`](https://probe.rs). The universal backend, beyond any single RTOS.
- **MCP for AI agents** — a stdio MCP server exposing `get_last_crash` plus board actions: `reboot_board`, `flash_firmware`, `read_memory`.

See [`engine/README.md`](engine/README.md) for how to run it.

## Status
🚧 Building toward the MVP on STM32 (Cortex-M) / Zephyr. The crash explainer works end to end and proved the thesis; the engine now reads live state and acts on the board over the debug probe. Next: richer live system state (threads, peripherals, variables). Other RTOSes and a built-in web agent come later.

## License
Apache-2.0
