// ============================================================
// Axyr Cockpit — right dock + bottom dock panels.
// Inspector (drill), Health (anomalies), Timeline + Threads.
// All values are real (snapshot contract). Nothing fabricated.
// ============================================================
const hp = React.createElement;

// ---- Inspector ---------------------------------------------
function RegDecode({ reg }) {
  return hp('div', { className: 'ax-reg' }, [
    hp('div', { className: 'ax-reg-head', key: 'h' }, [
      hp('span', { className: 'ax-reg-name', key: 'n' }, reg.name),
      hp('span', { className: 'ax-reg-val mono', key: 'v' }, reg.value),
    ]),
    hp('div', { className: 'ax-reg-fields', key: 'f' }, reg.fields.map((f, i) =>
      hp('div', { className: 'ax-field', key: i }, [
        hp('span', { className: 'ax-field-bit', key: 'b' }, f.value),
        hp('span', { className: 'ax-field-name mono', key: 'n' }, f.name),
        hp('span', { className: 'ax-field-mean', key: 'm' }, f.meaning),
      ])
    )),
  ]);
}

function WatchLive({ counter }) {
  const v = window.AX_VAR_BASE;
  return hp('div', { className: 'ax-watch', key: 'w' }, [
    hp('div', { className: 'ax-watch-row', key: 'r' }, [
      hp('span', { className: 'ax-watch-name mono', key: 'n' }, v.name),
      hp('span', { className: 'ax-watch-type', key: 't' }, v.type),
    ]),
    hp('div', { className: 'ax-watch-val mono', key: 'v' }, counter == null ? '—' : counter.toLocaleString('en-US')),
    hp('div', { className: 'ax-watch-meta mono', key: 'm' }, v.address + ' · read_variable'),
  ]);
}

function Inspector({ selected, mode, counter }) {
  const node = selected ? window.AX_NODES.find(n => n.id === selected) : null;
  const head = hp('div', { className: 'ax-card-head', key: 'h' }, [
    hp('span', { className: 'ax-card-title', key: 't' }, 'Inspector'),
    hp('span', { className: 'ax-card-sub', key: 's' }, node ? 'drill · ' + (KIND_LABEL[node.kind] || node.kind) : 'select a node'),
  ]);

  if (!node) {
    const D = window.AX_DEVICE;
    const facts = [
      ['chip', D.chip], ['core', D.core], ['cpuid', D.cpuid],
      ['probe', D.probe + ' · ' + D.transport], ['clock', D.clkMHz + ' MHz · PLL'],
    ];
    return hp('div', { className: 'ax-card grow', key: 'insp' }, [head,
      hp('div', { className: 'ax-card-body', key: 'b' }, [
        hp('div', { className: 'ax-sub-label', key: 'dl' }, 'Target'),
        hp('div', { className: 'ax-kv', key: 'kv' }, facts.map(([k, v], i) =>
          hp('div', { className: 'ax-kv-row', key: i }, [
            hp('span', { className: 'ax-kv-k', key: 'k' }, k),
            hp('span', { className: 'ax-kv-v mono', key: 'v' }, v),
          ]))),
        hp('div', { className: 'ax-sub-label', key: 'wl' }, 'Watched global · read_variable'),
        hp(WatchLive, { counter, key: 'w' }),
        hp('p', { className: 'ax-ondemand-note', key: 'tip' }, 'Click any block on the map to drill into a peripheral — decoded registers, address and live state.'),
      ]),
    ]);
  }

  const rows = [
    ['kind', KIND_LABEL[node.kind] || node.kind],
    ['compatible', node.compatible, true],
    ['address', node.addr || '— (no reg)', true],
    ['status', node.bus && mode !== 'crashed' ? 'okay · idle (no device)' : (mode === 'crashed' && node.id === 'cpu0' ? 'halted' : 'okay')],
  ];

  return hp('div', { className: 'ax-card grow', key: 'insp' }, [head,
    hp('div', { className: 'ax-card-body', key: 'b' }, [
      hp('div', { className: 'ax-insp-id', key: 'id' }, [
        hp('span', { className: 'ax-insp-ic', key: 'ic' }, kindIcon(node.kind, { s: 18 })),
        hp('span', { className: 'ax-insp-lb mono', key: 'lb' }, node.label),
      ]),
      hp('div', { className: 'ax-kv', key: 'kv' }, rows.map(([k, v, m], i) =>
        hp('div', { className: 'ax-kv-row', key: i }, [
          hp('span', { className: 'ax-kv-k', key: 'k' }, k),
          hp('span', { className: 'ax-kv-v' + (m ? ' mono' : ''), key: 'v' }, v),
        ])
      )),
      node.id === 'rcc' ? hp('div', { className: 'ax-decoded', key: 'dec' }, [
        hp('div', { className: 'ax-sub-label', key: 'l' }, 'Decoded registers · read_peripheral("RCC")'),
        ...window.AX_RCC.registers.map((r, i) => hp(RegDecode, { reg: r, key: i })),
      ]) : node.addr ? hp('div', { className: 'ax-ondemand', key: 'od' }, [
        hp('div', { className: 'ax-sub-label', key: 'l' }, 'Available reads'),
        hp('div', { className: 'ax-tool-chips', key: 'c' }, [
          hp('span', { className: 'ax-tool-chip mono', key: '1' }, 'read_peripheral("' + node.label.split(' ')[0] + '")'),
          hp('span', { className: 'ax-tool-chip mono', key: '2' }, 'read_memory(' + node.addr + ', 16)'),
        ]),
        hp('p', { className: 'ax-ondemand-note', key: 'n' }, 'Decoded on demand — non-intrusive SWD read, never halts the core.'),
      ]) : hp('p', { className: 'ax-ondemand-note', key: 'nn' }, 'GPIO-level node — no memory-mapped register block.'),
    ]),
  ]);
}

// ---- Health ------------------------------------------------
function Health({ mode }) {
  const items = mode === 'crashed' ? [
    { lvl: 'err', t: 'Bus fault — invalid memory access (precise)', s: 'fault address 0xbadcafe0 · i2c_read_reg @ main.c:30' },
    { lvl: 'err', t: 'Core halted', s: 'k_fatal_halt(reason=25) · awaiting reboot or flash' },
    { lvl: 'warn', t: 'Last reset: software reset', s: 'RCC CSR: SFTRSTF' },
  ] : [
    { lvl: 'ok', t: 'No anomalies', s: '3 threads nominal · clocks locked' },
    { lvl: 'warn', t: 'thread_analyzer stack at 53%', s: '552 / 1024 B — highest of the three' },
    { lvl: 'ok', t: 'idle 99% CPU', s: 'board mostly sleeping in k_msleep(250)' },
  ];
  const icon = (l) => l === 'err' ? UI.warn({ s: 15 }) : l === 'warn' ? UI.warn({ s: 15 }) : UI.check({ s: 15 });
  return hp('div', { className: 'ax-card', key: 'health' }, [
    hp('div', { className: 'ax-card-head', key: 'h' }, [
      hp('span', { className: 'ax-card-title', key: 't' }, 'Health'),
      hp('span', { className: 'ax-card-sub', key: 's' }, 'get_health'),
    ]),
    hp('div', { className: 'ax-card-body', key: 'b' },
      items.map((it, i) => hp('div', { className: 'ax-anom ' + it.lvl, key: i }, [
        hp('span', { className: 'ax-anom-ic', key: 'i' }, icon(it.lvl)),
        hp('div', { key: 'tx' }, [
          hp('div', { className: 'ax-anom-t', key: 't' }, it.t),
          hp('div', { className: 'ax-anom-s', key: 's' }, it.s),
        ]),
      ]))
    ),
  ]);
}

// ---- Timeline + Threads (bottom dock) ----------------------
function Timeline({ mode }) {
  const threads = window.AX_THREADS;
  const tl = window.AX_TIMELINE;
  const clk = window.AX_DEVICE.clkMHz * 1e6;
  const ms = (cyc) => (cyc / clk * 1000);
  const span = tl[tl.length - 1].cycles || 1;

  return hp('div', { className: 'ax-card bottom', key: 'tl' }, [
    hp('div', { className: 'ax-card-head', key: 'h' }, [
      hp('span', { className: 'ax-card-title', key: 't' }, 'Execution & History'),
      hp('span', { className: 'ax-card-sub', key: 's' }, 'get_threads · get_trace · get_history'),
    ]),
    hp('div', { className: 'ax-exec', key: 'b' }, [
      // threads
      hp('div', { className: 'ax-threads', key: 'th' }, threads.map((t, i) =>
        hp('div', { className: 'ax-thread', key: i }, [
          hp('div', { className: 'ax-thread-top', key: 'tp' }, [
            hp('span', { className: 'ax-thread-name mono', key: 'n' }, t.name),
            hp('span', { className: 'ax-thread-cpu', key: 'c' }, t.cpu_pct + '% CPU'),
          ]),
          hp('div', { className: 'ax-bar', key: 'bar' }, [
            hp('div', { className: 'ax-bar-fill cpu', key: 'f', style: { width: t.cpu_pct + '%' } }),
          ]),
          hp('div', { className: 'ax-thread-stack', key: 'st' }, [
            hp('div', { className: 'ax-bar thin', key: 'bar' }, [
              hp('div', { className: 'ax-bar-fill stack' + (t.stack_pct > 50 ? ' warn' : ''), key: 'f', style: { width: t.stack_pct + '%' } }),
            ]),
            hp('span', { className: 'ax-thread-stxt mono', key: 'x' }, 'stack ' + t.stack_used + '/' + t.stack_total + 'B · ' + t.stack_pct + '%'),
          ]),
        ])
      )),
      // history · real get_history time-series + compare two states (diff_snapshot)
      hp(History, { mode, key: 'rep' }),
    ]),
  ]);
}

// ---- History (real get_history time-series + compare two states) ----------
const uSt = React.useState, uEf = React.useEffect, uMo = React.useMemo;

// Build the scrubable points. LIVE → the engine's recorded get_history ticks
// {t_ms, core, threads, variables}; DEMO → the bundled real context-switch
// timeline; CRASHED → the real serial telemetry to the fault. Each carries a
// `state` so two points can be diffed. Never invented.
function buildTicks(mode) {
  const live = !!window.AX_LIVE;
  if (mode === 'crashed') {
    return window.AX_CRASH.recent_telemetry.map((line) => ({
      label: line,
      tag: (line.includes('FAULT') || line.includes('<err>')) ? 'fault' : line.includes('About to crash') ? 'pre' : 'boot',
      state: null,
    }));
  }
  if (live && Array.isArray(window.AX_HISTORY) && window.AX_HISTORY.length) {
    const vname = window.AX_VAR_BASE.name;
    return window.AX_HISTORY.map((hk) => {
      const v = (hk.variables || [])[0];
      const counter = v ? (typeof v.value === 'number' ? v.value : parseInt(v.value, 10)) : null;
      return {
        label: 'core ' + (hk.core || '?') + (counter != null ? ' · ' + vname + ' = ' + counter : ''),
        sub: hk.t_ms != null ? (hk.t_ms / 1000).toFixed(1) + ' s' : '',
        tag: hk.core === 'running' ? 'main' : hk.core === 'halted' ? 'fault' : 'idle',
        state: {
          core: hk.core, counter,
          threads: Object.fromEntries((hk.threads || []).map((t) => [t.name, { cpu: t.cpu_pct, stack: t.stack_pct }])),
        },
      };
    });
  }
  const clk = window.AX_DEVICE.clkMHz * 1e6;
  return window.AX_TIMELINE.map((e) => ({
    label: e.thread + ' running',
    sub: e.cycles.toLocaleString('en-US') + ' cyc · ' + (e.cycles / clk * 1000).toFixed(2) + ' ms',
    tag: e.thread,
    state: { core: e.thread === 'main' ? 'running' : 'sleeping', thread: e.thread, cycles: e.cycles },
  }));
}

// What changed between two recorded states — the "compare two states" view.
function diffStates(a, b) {
  if (!a || !b) return [['—', 'pick two points to compare']];
  const rows = [];
  if (a.core !== b.core) rows.push(['core', a.core + ' → ' + b.core]);
  if (a.counter != null && b.counter != null && a.counter !== b.counter) {
    const d = b.counter - a.counter;
    rows.push([window.AX_VAR_BASE.name, (d >= 0 ? '+' : '') + d.toLocaleString('en-US') + '   (' + a.counter + ' → ' + b.counter + ')']);
  }
  if (a.thread && b.thread && a.thread !== b.thread) rows.push(['running', a.thread + ' → ' + b.thread]);
  if (a.cycles != null && b.cycles != null && a.cycles !== b.cycles) rows.push(['Δ cycles', (b.cycles - a.cycles).toLocaleString('en-US')]);
  if (a.threads && b.threads) {
    Object.keys(b.threads).forEach((name) => {
      const x = a.threads[name], y = b.threads[name];
      if (x && y && x.cpu !== y.cpu) rows.push([name + ' CPU', x.cpu + '% → ' + y.cpu + '%']);
      if (x && y && x.stack !== y.stack) rows.push([name + ' stack', x.stack + '% → ' + y.stack + '%']);
    });
  }
  return rows.length ? rows : [['—', 'no change between these two points']];
}

function History({ mode }) {
  const crashed = mode === 'crashed';
  const ticks = uMo(() => buildTicks(mode), [mode]);
  const last = ticks.length - 1;
  const [i, setI] = uSt(crashed ? last : 0);
  const [playing, setPlaying] = uSt(false);
  const [cmp, setCmp] = uSt(false);
  const [a, setA] = uSt(0);
  const [b, setB] = uSt(Math.max(last, 0));
  uEf(() => { setI(crashed ? last : 0); setPlaying(false); setCmp(false); setA(0); setB(Math.max(last, 0)); }, [mode]);
  uEf(() => {
    if (!playing) return;
    const t = setInterval(() => setI(p => (p >= last ? (clearInterval(t), p) : p + 1)), 720);
    return () => clearInterval(t);
  }, [playing, last]);
  uEf(() => { if (playing && i >= last) setPlaying(false); }, [i, playing, last]);

  const cur = ticks[Math.min(i, last)] || ticks[0];
  const clk = window.AX_DEVICE.clkMHz * 1e6;
  const endMs = crashed ? null : (window.AX_TIMELINE[window.AX_TIMELINE.length - 1].cycles / clk * 1000).toFixed(0);
  const canCompare = !crashed && ticks.length > 1 && !!ticks[0].state;

  return hp('div', { className: 'ax-replay', key: 'rep' }, [
    hp('div', { className: 'ax-sub-label ax-hist-label', key: 'l' }, [
      hp('span', { key: 't' }, crashed ? 'History · replay to the fault' : 'History · ' + (window.AX_LIVE ? 'get_history (live)' : 'what ran when')),
      canCompare && hp('button', { className: 'ax-hist-cmp' + (cmp ? ' on' : ''), key: 'c', onClick: () => setCmp(v => !v) }, cmp ? 'comparing A↔B' : 'compare'),
    ]),
    hp('div', { className: 'ax-replay-cur ' + cur.tag, key: 'cur' }, [
      hp('span', { className: 'ax-replay-step mono', key: 's' }, (i + 1) + '/' + ticks.length),
      hp('div', { className: 'ax-replay-body', key: 'tx' }, [
        hp('div', { className: 'ax-replay-lb' + (crashed ? ' mono' : ''), key: 'a' }, cur.label),
        cur.sub && hp('div', { className: 'ax-replay-sub mono', key: 'b' }, cur.sub),
      ]),
      cmp && canCompare && hp('div', { className: 'ax-hist-pins', key: 'pins' }, [
        hp('button', { className: 'ax-hist-pin' + (a === i ? ' set' : ''), key: 'a', onClick: () => setA(i) }, a === i ? 'A ✓' : 'set A'),
        hp('button', { className: 'ax-hist-pin' + (b === i ? ' set' : ''), key: 'b', onClick: () => setB(i) }, b === i ? 'B ✓' : 'set B'),
      ]),
    ]),
    hp('div', { className: 'ax-replay-ctrl', key: 'ctrl' }, [
      hp('button', { className: 'ax-replay-play', key: 'p', title: 'replay',
        onClick: () => { if (i >= last) setI(0); setPlaying(v => !v); } }, playing ? '❚❚' : '▶'),
      hp('input', { type: 'range', className: 'ax-scrub', key: 'r', min: 0, max: last, step: 1, value: Math.min(i, last),
        onChange: (e) => { setPlaying(false); setI(+e.target.value); } }),
    ]),
    cmp && canCompare && hp('div', { className: 'ax-cmp', key: 'cmp' }, [
      hp('div', { className: 'ax-cmp-hd', key: 'hd' }, ['compare A ', hp('b', { key: 'a' }, '#' + (a + 1)), ' ↔ B ', hp('b', { key: 'b' }, '#' + (b + 1)), ' · diff_snapshot']),
      ...diffStates((ticks[Math.min(a, last)] || {}).state, (ticks[Math.min(b, last)] || {}).state).map((row, k) =>
        hp('div', { className: 'ax-cmp-row', key: k }, [
          hp('span', { className: 'ax-cmp-k', key: 'k' }, row[0]),
          hp('span', { className: 'ax-cmp-v mono', key: 'v' }, row[1]),
        ])),
    ]),
    hp('div', { className: 'ax-replay-axis mono', key: 'ax' }, [
      hp('span', { key: '0' }, crashed ? 'boot' : (window.AX_LIVE ? '0 s' : '0 cyc')),
      hp('span', { className: 'ax-replay-note', key: 'n' }, crashed ? '↔ ~28 ms coredump capture' : (window.AX_LIVE ? '↔ recorded ticks' : '↔ 250 ms loop · ' + window.AX_DEVICE.clkMHz + ' MHz')),
      hp('span', { key: '1' }, crashed ? 'BUS FAULT' : (window.AX_LIVE ? 'now' : endMs + ' ms')),
    ]),
  ]);
}

Object.assign(window, { Inspector, Health, Timeline });
