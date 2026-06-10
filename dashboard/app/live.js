// ============================================================
// Axyr Cockpit — live engine link.
// Talks to the engine's HTTP API (the same dispatch as MCP):
//   GET /snapshot, GET /graph, GET /health, POST /reboot, POST /flash
// On load it HYDRATES the window.AX_* globals from the real system, so the
// cockpit opens on the actual board — never invented data. If the engine
// isn't reachable, the bundled real demo snapshot is used unchanged.
//
// Engine URL: ?engine=http://host:port  ·  or localStorage.AXYR_ENGINE
//             ·  default http://127.0.0.1:7878
// ============================================================
(function () {
  const base = () =>
    new URLSearchParams(location.search).get('engine') ||
    localStorage.getItem('AXYR_ENGINE') ||
    'http://127.0.0.1:7878';

  async function get(path) {
    const r = await fetch(base() + path, { headers: { accept: 'application/json' } });
    if (!r.ok) throw new Error(path + ' → ' + r.status);
    return r.json();
  }

  // The engine's get_system_map `kind` → the cockpit's icon `kind`.
  const KIND_MAP = {
    'i2c bus': 'i2c', 'spi bus': 'spi', 'can bus': 'i2c',
    core: 'core', ram: 'ram', clock: 'clock', sensor: 'sensor', led: 'led',
    button: 'button', uart: 'uart', adc: 'adc', dac: 'adc', pwm: 'pwm',
    timer: 'timer', rtc: 'rtc', watchdog: 'watchdog', gpio: 'gpio', dma: 'gpio',
    flash: 'flash', usb: 'usb', header: 'header', irq: 'irq', device: 'gpio',
  };
  const mapKind = (id, k) => (id === 'soc' ? 'soc' : KIND_MAP[k] || 'gpio');

  // /graph {board, nodes:[{id,kind,address,status}], edges:[{from,to}], disabled}
  // → the cockpit's node list (kind drives the icon, tier the layout).
  function nodesFromGraph(g) {
    const parent = {};
    (g.edges || []).forEach((e) => { parent[e.to] = e.from; });
    const tierOf = (id) => {
      if (id === 'soc') return 'hub';
      const p = parent[id];
      if (p === 'board' || p == null) return 'board';
      if (p === 'soc') return 'soc';
      return 'leaf';
    };
    // Infrastructure leaves the devicetree carries but that clutter a system
    // overview: flash partitions, raw oscillator sources, NVIC/SysTick, etc.
    // Flagged `noise` → folded out of the schematic (still in the raw graph).
    const NOISE = /(_partition$)|nvmem|layout|^clk_|^rctl$|^stop$|^vref$|^vbat$|^nvic$|^systick$/i;
    return (g.nodes || [])
      .filter((n) => n.id !== 'board') // synthetic root, not a device
      .map((n) => ({
        id: n.id,
        label: n.id,
        kind: mapKind(n.id, n.kind),
        tier: tierOf(n.id),
        addr: n.address || null,
        bus: /bus/.test(n.kind || ''),
        compatible: n.compatible || n.kind || '',
        noise: tierOf(n.id) === 'leaf' || NOISE.test(n.id),
      }));
  }

  // The honestly-active nodes when running: the CPU runs, the watched global
  // lives in RAM, the clocks are locked, code is fetched from flash. Derived
  // from the real node set — never a hard-coded list, never invented traffic.
  function deriveActive(nodes) {
    const pick = (pred) => nodes.filter((n) => pred(n) && !n.noise).map((n) => n.id);
    return [
      ...pick((n) => n.kind === 'core'),
      ...pick((n) => n.kind === 'ram'),
      ...pick((n) => n.kind === 'clock'),
      ...pick((n) => n.kind === 'flash'),
    ];
  }

  // Pull a few decoded fields out of the snapshot, defensively — every field
  // falls back to the bundled demo value so a partial snapshot never breaks.
  function hydrateFromSnapshot(s) {
    const dev = s.device || {};
    Object.assign(window.AX_DEVICE, {
      board: dev.board || window.AX_DEVICE.board,
      vendor: dev.vendor || window.AX_DEVICE.vendor,
      chip: dev.chip || window.AX_DEVICE.chip,
      core: dev.core || window.AX_DEVICE.core,
      cpuid: dev.cpuid || window.AX_DEVICE.cpuid,
      probe: dev.probe || window.AX_DEVICE.probe,
      transport: dev.transport || window.AX_DEVICE.transport,
    });

    if (Array.isArray(s.threads) && s.threads.length) {
      window.AX_THREADS = s.threads.map((t) => ({
        name: t.name,
        stack_used: t.stack_used ?? 0,
        stack_total: t.stack_total ?? 0,
        stack_pct: t.stack_pct ?? (t.stack_total ? Math.round((t.stack_used / t.stack_total) * 100) : 0),
        cpu_pct: t.cpu_pct ?? 0,
      }));
    }
    if (Array.isArray(s.timeline) && s.timeline.length) {
      window.AX_TIMELINE = s.timeline.map((e) => ({ cycles: e.cycles ?? 0, thread: e.thread || '?' }));
    }
    const v = (s.variables || [])[0];
    if (v) {
      window.AX_VAR_BASE = { name: v.name, address: v.address || '', type: v.type || 'uint32' };
      window.AX_LIVE_COUNTER = typeof v.value === 'number' ? v.value : parseInt(v.value, 10);
    }
    if (s.crash && s.crash.cause) {
      const c = s.crash;
      window.AX_CRASH = {
        cause: c.cause,
        reason_code: c.reason_code,
        fault_address: c.fault_address || '',
        location: c.location || window.AX_CRASH.location,
        call_stack: c.call_stack || window.AX_CRASH.call_stack,
        registers: c.registers || window.AX_CRASH.registers,
        recent_telemetry: c.recent_telemetry || window.AX_CRASH.recent_telemetry,
      };
    }
    const st = s.state || {};
    const mode = st.core === 'crashed' || (s.crash && s.crash.cause) ? 'crashed' : 'running';
    if (st.summary) window.AX_STATE[mode].summary = st.summary;
    if (st.reset_reason) window.AX_STATE[mode].reset_reason = st.reset_reason;
    if (st.clocks) window.AX_STATE[mode].clocks = st.clocks;
    return mode;
  }

  // What the engine detected (board / probe / build) for the Connect screen,
  // or null if no agent is reachable.
  async function connectInfo() {
    try { return await get('/connect'); } catch (e) { return null; }
  }

  // Fetch /snapshot + /graph + /history and populate the globals before render.
  async function hydrate() {
    const [snap, graph, history] = await Promise.all([
      get('/snapshot').catch(() => null),
      get('/graph').catch(() => null),
      get('/history').catch(() => null),
    ]);
    if (!snap && !graph) throw new Error('engine offline');
    if (graph && Array.isArray(graph.nodes) && graph.nodes.length) {
      window.AX_NODES = nodesFromGraph(graph);
      window.AX_ACTIVE_RUNNING = deriveActive(window.AX_NODES);
      if (Array.isArray(graph.disabled)) {
        window.AX_DISABLED = graph.disabled.slice(0, 12);
        window.AX_DISABLED_TOTAL = graph.disabled.length;
      }
    }
    if (Array.isArray(history) && history.length) window.AX_HISTORY = history;
    window.AX_LIVE_MODE = snap ? hydrateFromSnapshot(snap) : 'running';
    window.AX_LIVE = true;
  }

  // Poll /snapshot and broadcast {mode, counter} so the app stays live without
  // re-mounting (preserves selection, open tabs, etc.).
  let timer = null;
  function startPolling(intervalMs = 1000) {
    if (timer) clearInterval(timer);
    const tick = async () => {
      try {
        const s = await get('/snapshot');
        const v = (s.variables || [])[0];
        const counter = v ? (typeof v.value === 'number' ? v.value : parseInt(v.value, 10)) : null;
        const mode = (s.state && s.state.core === 'crashed') || (s.crash && s.crash.cause) ? 'crashed' : 'running';
        window.dispatchEvent(new CustomEvent('axyr:tick', { detail: { mode, counter } }));
      } catch (e) { /* engine blip — keep last state */ }
    };
    timer = setInterval(tick, intervalMs);
  }

  // Fire-and-report an engine action (reboot / flash), used by the status bar
  // when running live. Returns the engine's text reply.
  async function action(path, body) {
    const r = await fetch(base() + path, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: body ? JSON.stringify(body) : undefined,
    });
    return r.text();
  }

  window.AxLive = { base, connectInfo, hydrate, startPolling, action };
})();
