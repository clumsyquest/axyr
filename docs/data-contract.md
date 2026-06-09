# Axyr data contract

The contract between the Axyr engine and its consumers — the **dashboard** (human
UI) and **AI agents**. It defines one shape: the **system snapshot**, a JSON
object describing an embedded system's real state.

- Formal schema: [`system-snapshot.schema.json`](system-snapshot.schema.json)
  (JSON Schema 2020-12).
- Example (real capture): [`system-snapshot.example.json`](system-snapshot.example.json).

Build the dashboard against the schema; the example shows a populated snapshot.

## Status

`schema_version` is **1.0**. The snapshot shape is stable; changes are additive
(new optional fields/sections). Today the engine exposes each section through an
MCP tool returning human text (see `actions`); the canonical machine form is this
JSON. A single `get_snapshot` tool returning the whole object is the intended
one-call endpoint for the dashboard (next implementation step) — the shape it
returns is exactly this contract.

## Top-level

| Field | Type | Notes |
|---|---|---|
| `schema_version` | string | `"1.0"`. |
| `captured_at` | string | ISO-8601 UTC. |
| `device` | object | Target identity. |
| `state` | object | Current core state. |
| `system_map` | object | Static hardware map. |
| `threads` | array | Live RTOS threads (may be empty). |
| `timeline` | array | Recent context switches. |
| `variables` | array | Watched globals. |
| `peripherals` | array | Decoded peripheral state (on demand). |
| `crash` | object \| null | Last crash post-mortem. |
| `actions` | array | What an agent can do (MCP tools). |

## Sections

- **device** — `board`, `chip`, `core`, `cpuid`, `probe`, `transport`.
- **state** — `core` (`running`/`sleeping`/`halted`/`crashed`/`unknown`),
  `summary`, `reset_reason` (from RCC flags), `clocks`.
- **system_map** — `source`, `devices` (a tree of `{label, kind, compatible,
  address, status, children}`), `disabled`. `kind` is a human category (`led`,
  `sensor`, `i2c bus`, `uart`, `gpio`, `clock`, `core`, …). Addresses are hex
  strings; `null` when the node has none.
- **threads** — per thread: `name`, `stack_used`/`stack_total`/`stack_pct`
  (bytes/%), `cpu_pct`. Empty on bare-metal.
- **timeline** — context switches oldest-first: `{cycles, thread}` ("what ran
  when"). `cycles` are CPU cycles.
- **variables** — watched globals: `{name, address (hex), type, value}`.
- **peripherals** — decoded registers: `{name, address, registers:[{name, value
  (hex), fields:[{name, value, meaning}]}]}`. `meaning` is the SVD symbolic name
  when defined. Read-side-effect registers are omitted (non-intrusive).
- **crash** — `cause`, `reason_code`, `fault_address`, `location`
  (`{function,file,line}`), `call_stack` (frames), `registers`,
  `recent_telemetry`. `null` if no crash recorded.
- **actions** — `{name, kind: read|action, args}` for each MCP tool.

## Modularity (OS-agnostic core vs backend)

The snapshot shape is stable across OSes/boards/architectures; only the
*producers* differ per backend:

- **OS-agnostic** (any RTOS / bare-metal): `device`, `state.clocks`/`reset_reason`,
  `peripherals`, `variables`, `crash.registers`, `actions`. These come from the
  silicon (SWD/SVD/ELF).
- **Backend-provided** (Zephyr today; FreeRTOS/bare-metal later may fill these
  differently or leave them empty): `system_map` (devicetree), `threads`
  (thread analyzer), `timeline` (tracing), `crash` call-stack symbolication.

A consumer should treat backend-provided arrays as possibly empty and render
what's present.

## Guarantees

- **Non-intrusive**: producing a snapshot never halts the core (RTT is buffered;
  SWD reads run in background; the coredump is read from RAM after a fault).
- **Deterministic**: transient probe glitches are retried; values reflect the
  real device state at `captured_at`.
