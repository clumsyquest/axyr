# Axyr Dashboard

The web cockpit: a real-time, human-readable view of the microcontroller — the
same system state the AI agent sees over MCP, rendered for a person.

It is **not** a raw-data dump. It translates what the chip is doing right now: the
board's own peripherals laid out from the devicetree, the live data flow, and —
when something breaks — exactly **where** (which device, which file:line:function).

## What's here

A single app with two spaces (left rail):

- **System** — the living board. A schematic "backplane" built automatically from
  the devicetree (`soc` as the central bus matrix, peripherals grouped into
  functional blocks by `kind`, animated dataflow on active links, open bus ports,
  a collapsible shelf of disabled peripherals). Plus an **Inspector** (drill into
  a node → decoded registers, address, live value), **Health** (anomalies in plain
  language), and **Execution & History** (threads, context-switch timeline, a
  replay scrubber). On a crash, a **Crash** panel takes over — cause, clickable
  `file:line`, call stack, fault registers, real serial telemetry.
- **Agent** — a Claude-Code-style workspace: the project file tree, a code editor
  with the crash line highlighted, and a chat where the agent reads the system via
  the real tools (`get_last_crash`, `read_memory`, `flash_firmware`…), explains,
  proposes a patch, and flashes/reboots.

The map **adapts to any project**: blocks are derived from each node's devicetree
`kind`, so a new board with different peripherals lays itself out — nothing is
hard-coded per board.

## How a developer connects a board

Only one thing is installed on the dev's machine: the **`axyr` agent** (the engine
binary) — a thin local program that drives the debug probe over USB (the cloud
can't reach USB). Everything else (the dashboard, later the cloud engine) is web.

1. Run the agent from your firmware project — **zero config**:
   ```bash
   axyr            # auto-detects the probe + the build (build/zephyr/zephyr.elf) + the chip
   ```
   It prints what it found and serves the dashboard API on `127.0.0.1:7878`.
2. Open the dashboard. The **Connect screen** shows what was detected —
   *"🟢 STM32F401RE-NUCLEO via ST-LINK · id 0x433 · build …"* — and you click
   **Connect**. One click. If no agent is running, the screen shows the install
   steps and an "explore the demo" link instead.

The agent identifies the chip straight from the silicon (CPUID + STM32 IDCODE),
finds the build artifacts in your own project, and needs no env vars. (The cloud
sign-in / hosted relay is the next roadmap step; today the agent + dashboard run
locally.)

## Running it (dashboard alone)

No build step. Serve the folder statically and open it:

```bash
cd dashboard
python3 -m http.server 8099
# open http://127.0.0.1:8099   → Connect screen (detects the agent, or offers the demo)
```

(React is loaded from a CDN; the components use `React.createElement` directly, so
there's no bundler or JSX transpile step.)

### Live vs demo data

- **Live** — point it at a running engine (its HTTP API, default
  `http://127.0.0.1:7878`). On load the dashboard hydrates from the engine's
  `GET /snapshot` and `GET /graph`, then polls for the live `axyr_counter` and
  running/crashed state. Reboot/flash buttons call the engine. The status bar
  shows `· live`.

  ```
  http://127.0.0.1:8099/?engine=http://127.0.0.1:7878
  ```
  (or set `localStorage.AXYR_ENGINE`). The engine serves permissive CORS, so the
  dashboard can run on a different origin.

- **Demo** — if no engine is reachable, it falls back to the bundled snapshot.
  That data is **real**, taken verbatim from the repo (the two demo firmwares:
  `live_demo` running with `axyr_counter`, and `crash_demo`'s bus fault at
  `0xBADCAFE0` → `i2c_read_reg` → `read_sensor` → `main`). Nothing is invented.

The interface is responsive — it reflows cleanly from desktop to tablet to phone
(the rail moves to the bottom, panels stack, the schematic collapses to a column).

## Layout

```
dashboard/
  index.html      — shell: fonts, styles, scripts, mount (live-hydrate → render)
  styles.css      — the full theme (light "paper", IBM Plex Sans/Mono)
  app/
    data.js       — the bundled real snapshot (demo fallback)
    icons.js      — one geometric icon per devicetree kind, + UI glyphs
    map.js        — the System map (kind-driven backplane schematic)
    panels.js     — Inspector, Health, Execution & History
    crash.js      — the crash takeover panel
    files.js      — real firmware source (Agent file tree / editor)
    agent.js      — the Agent workspace (tree + editor + chat)
    live.js       — engine HTTP-API link (hydrate, poll, actions)
    app.js        — app shell, rail, status bar, state
```

The design is the "Cockpit" direction iterated in Claude Design, implemented here
for real and wired to the engine API. It is the human half of Axyr's promise; the
machine half is the same data over MCP.
