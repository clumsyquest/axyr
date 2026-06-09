# axyr-engine

The local agent: it owns the debug probe, reads the board's telemetry and crashes
over **RTT** (no serial port), turns a crash into a human-readable post-mortem
(cause, source location, **full call stack**, recent telemetry), and exposes the
system to AI agents over MCP — including board actions.

## Layout

- `src/lib.rs` — crash logic: parse the `AXYR_CRASH` line, decode the fatal
  reason, symbolize the PC via `addr2line`, format the report. Pure and unit
  tested (no hardware needed: `cargo test`).
- `src/coredump.rs` — capture the Zephyr coredump block from the telemetry stream
  and resolve it into a call stack by driving Zephyr's tooling + GDB offline.
- `src/recent_log.rs` — a ring buffer of recent telemetry, attached to the report
  as "what was happening just before the crash".
- `src/probe.rs` — debug-probe (SWD) access via `probe-rs`: attach, read
  memory/registers, reset, flash.
- `src/rtt.rs` — non-intrusive telemetry over SEGGER RTT: drain the target's
  up-channel without ever halting the core. The transport that replaces serial.
- `src/system_map.rs` — parses the Zephyr devicetree (`zephyr.dts`) into the
  board's hardware map: peripherals, sensors/actuators, addresses, on/off state.
- `src/symbols.rs` — resolves a global variable to its address/size from the
  firmware ELF symbol table (generic: any ELF, any variable).
- `src/threads.rs` — parses Zephyr thread-analyzer telemetry into per-thread
  stack usage and CPU load (live RTOS state).
- `src/trace.rs` — reconstructs the context-switch timeline from a RAM ring buffer (what ran when).
- `src/peripheral.rs` — decodes a peripheral's live registers into plain language
  via the chip's SVD (skips side-effect/write-only registers — non-intrusive).
  Covers clocks, reset reason (RCC CSR), enabled peripherals, GPIO, NVIC, etc.
- `src/agent.rs` — the probe-owner loop: one thread owns the probe, drains RTT
  telemetry (feeding the crash pipeline), and runs queued actions between drains.
- `src/main.rs` — wires it together: spawns the agent thread and serves MCP over
  stdio (`get_last_crash`, `get_system_map`, `get_threads`, `get_trace`, `get_snapshot`, `get_health`, `diff_snapshot`, `get_history`, `list_variables`, `list_peripherals`, `list_firmware`, `read_variable`, `read_peripheral`,
  `reboot_board`, `flash_firmware`, `read_memory`, `watch_until`). Reads are served directly; actions are routed to the agent
  thread so they never disturb the real-time telemetry.
- `src/bin/rtt_read.rs` — streams the firmware's RTT output (`rtt_read [chip]`).
- `src/bin/probe_check.rs` — hardware smoke test for the probe (attach / read /
  reset / flash).

## One owner of the probe (concurrency)

The SWD link is a single, inherently serial resource, so exactly one thread owns
the probe and serializes everything: it drains RTT continuously and runs actions
between drains. Because RTT is buffered on the target, an action only pauses the
host's draining for its duration — no telemetry is lost and the target's
real-time behaviour is never disturbed.

## Build & test

```bash
cargo build
cargo test
```

## Run

```bash
axyr-engine <elf> <addr2line> <crash-file>
```

Example:

```bash
export AXYR_CHIP=STM32F401RETx            # target chip (default)
export AXYR_DTS=~/zephyrproject/zephyr/build/zephyr/zephyr.dts   # for get_system_map
export AXYR_SVD=~/STMicroelectronics/.../SVD/STM32F401.svd       # for read_peripheral
# coredump tooling, to resolve the full call stack:
export AXYR_GDB=~/zephyr-sdk-1.0.1/gnu/arm-zephyr-eabi/bin/arm-zephyr-eabi-gdb
export AXYR_COREDUMP_LOG_PARSER=~/zephyrproject/zephyr/scripts/coredump/coredump_serial_log_parser.py
export AXYR_COREDUMP_GDBSERVER=~/zephyrproject/zephyr/scripts/coredump/coredump_gdbserver.py

axyr-engine \
  ~/zephyrproject/zephyr/build/zephyr/zephyr.elf \
  ~/zephyr-sdk-1.0.1/gnu/arm-zephyr-eabi/bin/arm-zephyr-eabi-addr2line \
  /tmp/last_crash.txt
```

If the coredump vars are unset, the report still has cause + location + recent
telemetry, just no call stack. Requires `pyelftools` for the coredump scripts.

### Coredump pipeline

The firmware uses Zephyr's **IN_MEMORY coredump backend**: a fault writes the
full dump to a RAM buffer (`in_memory_coredump`) instead of streaming it through
the log subsystem. When the agent sees the `AXYR_CRASH` line, it resolves that
buffer's address from the ELF, reads it in one SWD block (the core is halted
after the fault), and drives GDB offline (`coredump_gdbserver.py --pipe`) on the
raw bytes to get the `bt` frames. No log subsystem, no hex, no streaming — crash
capture dropped from tens of seconds to ~3s (now dominated by the GDB spawn).

## Live state (variables)

Polling an arbitrary global over SWD only works while the core is awake; during
sleep (WFI) the bus reads zeros, and halting to read is intrusive (unacceptable).
The deterministic path is a tiny low-priority on-device thread that samples the
requested addresses and pushes them over RTT — non-intrusive and reliable.
`symbols.rs` resolves the names; this on-device sampler is the next step.

## Connecting to the board on Linux

The ST-LINK's SWD link can wedge after repeated resets. What helps:

- Connect / flash **under reset**: `st-info --probe --connect-under-reset`,
  `st-flash --connect-under-reset ...`.
- Trigger a clean re-run with `openocd -f interface/stlink.cfg -f
  target/stm32f4x.cfg -c "init; reset run; exit"` (plain `st-flash reset` can
  leave the core halted).
- If the link wedges, USB-reset the ST-LINK (an `USBDEVFS_RESET` ioctl on the
  device under `/dev/bus/usb/...`) to make Linux re-enumerate it.
