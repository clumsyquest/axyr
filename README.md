# Axyr

The "DevTools / F12 for microcontrollers": an open-source layer that captures a microcontroller's real internal state in real time and makes it readable — for developers and for AI agents (via MCP).

## Why
Debugging embedded systems often means flying blind: a board reboots, a HardFault fires, and you're left guessing. Axyr turns that opaque failure into a clear, structured explanation — cause, function, file:line, registers — exposed both to humans and to AI agents.

## Structure
- `firmware/`  — on-device code (C, Zephyr) running on the chip
- `engine/`    — host engine + MCP server (Rust)
- `dashboard/` — web interface (TypeScript)
- `docs/`      — design notes and documentation

## Status
🚧 Pre-v1. First target: the "crash explainer" for STM32 (Cortex-M) on Zephyr.

## License
Apache-2.0
