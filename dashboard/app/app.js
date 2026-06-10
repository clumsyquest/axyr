// ============================================================
// Axyr Cockpit — app shell. Rail nav (System / Agent),
// status bar, state machine (running ⇄ crashed), live counter,
// crash → agent handoff.
// ============================================================
const hA = React.createElement;
const RS = React.useState, RE = React.useEffect, RR = React.useRef;

function StatusBar({ mode, onTriggerFault, onReboot, onFlash }) {
  const d = window.AX_DEVICE;
  const st = window.AX_STATE[mode];
  const crashed = mode === 'crashed';
  return hA('header', { className: 'ax-status' }, [
    hA('div', { className: 'ax-status-id', key: 'id' }, [
      hA('span', { className: 'ax-chip-ic', key: 'c' }, kindIcon('soc', { s: 18 })),
      hA('div', { key: 't' }, [
        hA('div', { className: 'ax-status-board', key: 'b' }, d.board),
        hA('div', { className: 'ax-status-meta mono', key: 'm' }, d.chip + ' · ' + d.core + ' · ' + d.cpuid),
      ]),
    ]),
    hA('div', { className: 'ax-status-state', key: 'state' }, [
      hA('span', { className: 'ax-state-badge ' + (crashed ? 'crashed' : 'running'), key: 'sb' }, [
        hA('span', { className: 'ax-state-led', key: 'l' }),
        crashed ? 'CRASHED' : 'RUNNING',
      ]),
      hA('div', { className: 'ax-state-detail', key: 'det' }, [
        hA('span', { className: 'ax-state-summary', key: 's' }, st.summary),
        hA('span', { className: 'ax-state-reset mono', key: 'r' }, 'reset: ' + st.reset_reason),
      ]),
    ]),
    hA('div', { className: 'ax-status-right', key: 'right' }, [
      hA('div', { className: 'ax-status-clk mono', key: 'clk' }, [
        hA('span', { className: 'ax-clk-dot', key: 'd' }), d.clkMHz + ' MHz · PLL'
      ]),
      hA('div', { className: 'ax-status-link mono', key: 'lk' }, d.probe + ' · ' + d.transport + (window.AX_LIVE ? ' · live' : ' · demo')),
      hA('div', { className: 'ax-status-btns', key: 'btn' }, [
        crashed
          ? hA('button', { className: 'ax-btn ghost', key: 'rb', onClick: onReboot }, [UI.reboot({ s: 15 }), 'reboot'])
          : hA('button', { className: 'ax-btn ghost', key: 'tf', onClick: onTriggerFault }, [UI.flash({ s: 15 }), 'trigger fault']),
        hA('button', { className: 'ax-btn', key: 'fl', onClick: onFlash }, [UI.flash({ s: 15 }), 'flash live_demo']),
      ]),
    ]),
  ]);
}

function Rail({ view, setView }) {
  const items = [['system', 'System', UI.map], ['agent', 'Agent', UI.agent]];
  return hA('nav', { className: 'ax-rail' }, [
    hA('div', { className: 'ax-logo', key: 'logo' }, [
      hA('span', { className: 'ax-logo-mark', key: 'm' }, 'A'),
    ]),
    hA('div', { className: 'ax-rail-items', key: 'items' }, items.map(([id, label, Icon]) =>
      hA('button', { className: 'ax-rail-btn' + (view === id ? ' active' : ''), key: id, onClick: () => setView(id), title: label }, [
        Icon({ s: 20 }),
        hA('span', { className: 'ax-rail-lb', key: 'l' }, label),
      ]))),
    hA('div', { className: 'ax-rail-foot mono', key: 'foot' }, 'v1'),
  ]);
}

function App() {
  // Live when the engine hydrated us (window.AX_LIVE); else demo simulation.
  const live = !!window.AX_LIVE;
  const [view, setView] = RS('system');
  const [mode, setMode] = RS(live ? (window.AX_LIVE_MODE || 'running') : 'running');
  const [selected, setSelected] = RS(null);
  const [openPath, setOpenPath] = RS(window.AX_OPEN_DEFAULT);
  const [activeLine, setActiveLine] = RS(null);
  const [kick, setKick] = RS(0);
  const [counter, setCounter] = RS(live && window.AX_LIVE_COUNTER != null ? window.AX_LIVE_COUNTER : 22);

  // LIVE: follow the engine's polled state (axyr_counter + running/crashed),
  // without re-mounting — preserves selection and open tabs.
  RE(() => {
    if (!live) return;
    const onTick = (e) => {
      if (e.detail.mode) setMode(e.detail.mode);
      if (e.detail.counter != null) setCounter(e.detail.counter);
    };
    window.addEventListener('axyr:tick', onTick);
    return () => window.removeEventListener('axyr:tick', onTick);
  }, [live]);

  // DEMO: replay live_demo's axyr_counter (++ every 250 ms) when not wired to a board.
  RE(() => {
    if (live || mode !== 'running') return;
    const t = setInterval(() => setCounter(c => c + 1), 250);
    return () => clearInterval(t);
  }, [mode, live]);

  const triggerFault = () => { setMode('crashed'); setSelected(null); };
  const reboot = () => { setMode('running'); if (live && window.AxLive) window.AxLive.action('/reboot').catch(() => {}); };
  const flash = () => { setMode('running'); if (!live) setCounter(22); };

  const openFromCrash = (path, line) => {
    setOpenPath(path); setActiveLine(line || null); setView('agent');
  };
  const handToAgent = () => { setView('agent'); setKick(k => k + 1); };

  return hA('div', { className: 'ax-app' }, [
    hA(Rail, { view, setView, key: 'rail' }),
    hA('div', { className: 'ax-main', key: 'main' }, [
      hA(StatusBar, { mode, onTriggerFault: triggerFault, onReboot: reboot, onFlash: flash, key: 'status' }),
      !window.AX_LIVE && hA('div', { className: 'ax-demo-banner', key: 'demo' }, [
        hA('span', { className: 'ax-demo-ic', key: 'i' }, UI.warn({ s: 15 })),
        hA('span', { key: 't' }, [
          hA('b', { key: 'b' }, 'Demo data'),
          ' — not connected to a board. This is the two real demo firmwares (live_demo / crash_demo), nothing live. To see your real system, run the engine and open ',
          hA('code', { key: 'c' }, '?engine=http://127.0.0.1:7878'),
          '.',
        ]),
      ]),
      view === 'system'
        ? hA('div', { className: 'ax-system', key: 'sys' }, [
            hA('div', { className: 'ax-sys-main', key: 'm' }, [
              hA(SystemMap, { mode, selected, onSelect: setSelected, counter, key: 'map' }),
              hA(Timeline, { mode, key: 'tl' }),
            ]),
            hA('div', { className: 'ax-sys-dock', key: 'dock' },
              mode === 'crashed'
                ? [hA(Crash, { onOpenFile: openFromCrash, onAskAgent: handToAgent, onReboot: reboot, key: 'crash' })]
                : [hA(Inspector, { selected, mode, counter, key: 'insp' }), hA(Health, { mode, key: 'health' })]
            ),
          ])
        : hA(AgentView, {
            mode, openPath, setOpenPath, activeLine, setActiveLine, kick,
            onFlash: flash, onReboot: reboot, key: 'agent',
          }),
    ]),
  ]);
}

// ---- Connect gate: detect the board, one click to go live ----------------
function Connect({ info, onConnect, onDemo, onRetry }) {
  const ready = info && info.ready;
  const probe = info && info.probe;
  const body = ready
    ? [
        hA('div', { className: 'ax-connect-detected', key: 'd' }, [
          hA('span', { className: 'ax-connect-led', key: 'l' }),
          hA('div', { key: 'tx' }, [
            hA('div', { className: 'ax-connect-board', key: 'b' }, info.board || info.chip),
            hA('div', { className: 'ax-connect-meta mono', key: 'm' },
              'via ' + (probe ? probe.name : 'debug probe') + (info.dev_id ? ' · id ' + info.dev_id : '')),
          ]),
        ]),
        info.build && info.build.elf && hA('div', { className: 'ax-connect-build mono', key: 'bd' }, 'build · ' + info.build.elf),
        hA('button', { className: 'ax-btn primary ax-connect-go', key: 'go', onClick: onConnect }, 'Connect'),
      ]
    : info
      ? [
          hA('div', { className: 'ax-connect-empty', key: 'e' }, 'Agent running — no board detected.'),
          hA('div', { className: 'ax-connect-hint', key: 'h' }, 'Plug your board’s USB (its on-board ST-LINK), then retry.'),
          hA('button', { className: 'ax-btn ghost', key: 'r', onClick: onRetry }, 'Retry'),
        ]
      : [
          hA('div', { className: 'ax-connect-empty', key: 'e' }, 'No Axyr agent detected on this machine.'),
          hA('div', { className: 'ax-connect-steps', key: 's' }, [
            hA('div', { className: 'ax-connect-step', key: '1' }, ['1 — install the agent', hA('code', { key: 'c' }, 'curl -fsSL https://get.axyr.dev | sh')]),
            hA('div', { className: 'ax-connect-step', key: '2' }, ['2 — run it in your firmware project', hA('code', { key: 'c' }, 'axyr')]),
            hA('div', { className: 'ax-connect-step', key: '3' }, ['3 — reload this page (it will detect your board)']),
          ]),
          hA('button', { className: 'ax-btn ghost', key: 'r', onClick: onRetry }, 'Retry'),
        ];
  return hA('div', { className: 'ax-connect' },
    hA('div', { className: 'ax-connect-card' }, [
      hA('div', { className: 'ax-connect-logo', key: 'lg' }, 'A'),
      hA('div', { className: 'ax-connect-title', key: 't' }, 'Connect your board'),
      ...body,
      hA('button', { className: 'ax-connect-demo', key: 'dm', onClick: onDemo }, 'or explore the demo →'),
    ]));
}

function Root() {
  const [phase, setPhase] = RS('connecting'); // connecting | gate | app
  const [info, setInfo] = RS(null);
  const [kick, setKick] = RS(0);

  RE(() => {
    let alive = true;
    setPhase('connecting');
    (async () => {
      const ci = window.AxLive ? await window.AxLive.connectInfo() : null;
      if (!alive) return;
      setInfo(ci);
      setPhase('gate');
    })();
    return () => { alive = false; };
  }, [kick]);

  const onConnect = async () => {
    if (window.AxLive) {
      try { await window.AxLive.hydrate(); window.AxLive.startPolling(); } catch (e) { /* fall through to whatever loaded */ }
    }
    setPhase('app');
  };
  const onDemo = () => { window.AX_LIVE = false; setPhase('app'); };
  const onRetry = () => setKick(k => k + 1);

  if (phase === 'connecting') {
    return hA('div', { className: 'ax-connect' }, hA('div', { className: 'ax-connect-card' }, hA('div', { className: 'ax-connect-spin' })));
  }
  if (phase === 'gate') return hA(Connect, { info, onConnect, onDemo, onRetry });
  return hA(App, null);
}

window.AxApp = Root;
