// ============================================================
// Axyr Cockpit — SystemMap v2: a clean "backplane" schematic.
// soc = central spine (bus matrix). Peripherals grouped into
// functional blocks, plugged into the spine with orthogonal
// connectors. Live dataflow packets on active links. Auto from
// the devicetree groups; nothing hard-coded per board.
// ============================================================
const { useState, useRef, useEffect, useLayoutEffect, useMemo, useCallback } = React;
const h = React.createElement;

// Functional blocks defined by devicetree `kind` (not hard-coded node ids), so
// the schematic builds itself from whatever the devicetree declares — any board,
// any peripheral set. Each non-`soc` node lands in the first block whose kinds
// include its kind; empty blocks are dropped.
const GROUP_DEFS = [
  { id: 'coremem', side: 'left',  label: 'CPU & Memory',    kinds: ['core', 'ram', 'flash'] },
  { id: 'clock',   side: 'left',  label: 'Clocks',          kinds: ['clock'] },
  { id: 'board',   side: 'left',  label: 'Board I/O',       kinds: ['led', 'button', 'sensor', 'usb', 'header', 'gpio'] },
  { id: 'bus',     side: 'right', label: 'Buses',           kinds: ['uart', 'i2c', 'spi'] },
  { id: 'timing',  side: 'right', label: 'Timers & Analog', kinds: ['timer', 'pwm', 'adc', 'rtc', 'dac'] },
  { id: 'system',  side: 'right', label: 'System',          kinds: ['watchdog', 'irq'] },
];
function buildGroups(nodes) {
  const fallback = GROUP_DEFS[GROUP_DEFS.length - 1].id;
  const groupOf = (kind) => (GROUP_DEFS.find(g => g.kinds.includes(kind)) || { id: fallback }).id;
  return GROUP_DEFS
    .map(def => ({
      id: def.id, side: def.side, label: def.label,
      ids: nodes.filter(n => n.id !== 'soc' && groupOf(n.kind) === def.id).map(n => n.id),
    }))
    .filter(g => g.ids.length);
}

function Chip({ n, selected, active, faulting, onSelect, refCb }) {
  const cls = 'ax-chip' + (selected ? ' sel' : '') + (active ? ' act' : '') + (faulting ? ' fault' : '');
  return h('button', { className: cls, ref: refCb || undefined, title: KIND_LABEL[n.kind] || n.kind,
    onClick: (e) => { e.stopPropagation(); onSelect(n.id); } }, [
    h('span', { className: 'ax-chip-ic', key: 'i' }, kindIcon(n.kind, { s: 16 })),
    h('span', { className: 'ax-chip-tx', key: 't' }, [
      h('span', { className: 'ax-chip-lb', key: 'l' }, n.label),
      n.addr && h('span', { className: 'ax-chip-ad', key: 'a' }, n.addr),
    ]),
    active && h('span', { className: 'ax-chip-led', key: 'led' }),
    n.bus && !active && !faulting && h('span', { className: 'ax-chip-port', key: 'p' }, [h('i', { key: 'i' }), 'open']),
  ]);
}

function SystemMap({ mode, selected, onSelect, counter }) {
  const nodeById = useMemo(() => Object.fromEntries(window.AX_NODES.map(n => [n.id, n])), []);
  const GROUPS = useMemo(() => buildGroups(window.AX_NODES), []);
  const [lines, setLines] = useState([]);
  const [fault, setFault] = useState(null);
  const [disOpen, setDisOpen] = useState(false);
  const mapRef = useRef(null), spineRef = useRef(null), cpuRef = useRef(null);
  const blockRefs = useRef({});

  const crashed = mode === 'crashed';
  const activeSet = crashed ? new Set() : new Set(window.AX_ACTIVE_RUNNING);

  // measure block edges → orthogonal connector geometry (mode-agnostic)
  const measure = useCallback(() => {
    const map = mapRef.current, spine = spineRef.current;
    if (!map || !spine) return;
    const mr = map.getBoundingClientRect(), sp = spine.getBoundingClientRect();
    const arr = [];
    GROUPS.forEach(g => {
      const el = blockRefs.current[g.id]; if (!el) return;
      const r = el.getBoundingClientRect();
      const y = r.top + r.height / 2 - mr.top;
      if (g.side === 'left') arr.push({ id: g.id, x1: r.right - mr.left, x2: sp.left - mr.left, y });
      else arr.push({ id: g.id, x1: r.left - mr.left, x2: sp.right - mr.left, y });
    });
    setLines(arr);
  }, []);

  useLayoutEffect(() => {
    measure();
    const ro = new ResizeObserver(measure);
    if (mapRef.current) ro.observe(mapRef.current);
    window.addEventListener('resize', measure);
    return () => { ro.disconnect(); window.removeEventListener('resize', measure); };
  }, [measure]);
  useEffect(() => { measure(); }, [disOpen, measure]);

  // fault marker geometry (recomputed once lines settle)
  useEffect(() => {
    if (!crashed || !cpuRef.current || !mapRef.current) { setFault(null); return; }
    const mr = mapRef.current.getBoundingClientRect();
    const c = cpuRef.current.getBoundingClientRect();
    setFault({ x1: c.right - mr.left, y1: c.top + c.height / 2 - mr.top, mx: mr.width / 2, my: 14 });
  }, [mode, lines]);

  const renderBlock = (g) => {
    const active = !crashed && g.ids.some(id => activeSet.has(id));
    const dim = crashed && !g.ids.includes('cpu0');
    return h('div', { className: 'ax-block' + (active ? ' active' : '') + (dim ? ' dim' : ''),
      key: g.id, ref: (el) => { blockRefs.current[g.id] = el; } }, [
      h('div', { className: 'ax-block-hd', key: 'hd' }, [
        h('b', { key: 'b' }, g.label),
        h('span', { className: 'ax-block-ct', key: 'c' }, g.ids.length),
      ]),
      h('div', { className: 'ax-block-chips' + (g.ids.length > 2 ? ' two' : ''), key: 'ch' }, g.ids.map(id =>
        h(Chip, { key: id, n: nodeById[id], selected: selected === id,
          active: !crashed && activeSet.has(id), faulting: crashed && id === 'cpu0',
          onSelect, refCb: id === 'cpu0' ? ((el) => { cpuRef.current = el; }) : null }))),
    ]);
  };

  const D = window.AX_DEVICE;
  return h('div', { className: 'ax-map' + (crashed ? ' crashed' : ''), ref: mapRef, onClick: () => onSelect(null) }, [
    h('div', { className: 'ax-map-top', key: 'top' }, [
      h('span', { className: 'ax-frame-tag', key: 't' }, [
        h('span', { className: 'ax-frame-dot', key: 'd' }), 'board · ', h('b', { key: 'b' }, D.board),
      ]),
      h('span', { className: 'ax-frame-src', key: 's' }, 'zephyr devicetree · get_system_map · get_history'),
    ]),
    // connectors
    h('svg', { className: 'ax-conns', key: 'svg' }, [
      ...lines.map(l => {
        const g = GROUPS.find(x => x.id === l.id);
        const live = !crashed && g.ids.some(id => activeSet.has(id));
        return h('line', { key: l.id, className: 'ax-conn' + (live ? ' live' : ''), x1: l.x1, y1: l.y, x2: l.x2, y2: l.y });
      }),
      ...lines.filter(l => { const g = GROUPS.find(x => x.id === l.id); return !crashed && g.ids.some(id => activeSet.has(id)); })
        .map(l => h('circle', { key: l.id + 'p', className: 'ax-conn-pkt', r: 3.2, cy: l.y, cx: l.x1 },
          h('animate', { key: 'a', attributeName: 'cx', from: l.x1, to: l.x2, dur: '1.3s', repeatCount: 'indefinite' }))),
      fault && h('line', { key: 'fault', className: 'ax-conn fault', x1: fault.x1, y1: fault.y1, x2: fault.mx, y2: fault.my + 34 }),
    ]),
    // schematic
    h('div', { className: 'ax-schem', key: 'schem' }, [
      h('div', { className: 'ax-col left', key: 'l' }, GROUPS.filter(g => g.side === 'left').map(renderBlock)),
      h('div', { className: 'ax-spine' + (crashed ? ' halted' : ''), key: 'sp', ref: spineRef }, [
        h('div', { className: 'ax-spine-ic', key: 'i' }, kindIcon('soc', { s: 26 })),
        h('div', { className: 'ax-spine-lb', key: 'l' }, 'soc'),
        h('div', { className: 'ax-spine-sub', key: 's' }, 'STM32F401'),
        h('div', { className: 'ax-spine-tag', key: 't' }, 'BUS MATRIX'),
        h('div', { className: 'ax-spine-state', key: 'st' }, [h('i', { key: 'd' }), crashed ? 'halted' : 'running']),
      ]),
      h('div', { className: 'ax-col right', key: 'r' }, GROUPS.filter(g => g.side === 'right').map(renderBlock)),
    ]),
    // fault marker
    fault && h('div', { className: 'ax-fault-tag', key: 'ft', style: { left: fault.mx, top: fault.my } }, [
      h('span', { className: 'ax-fault-x', key: 'x' }, UI.close({ s: 13 })),
      h('div', { key: 't' }, [
        h('div', { className: 'ax-fault-addr', key: 'a' }, '0xBADCAFE0'),
        h('div', { className: 'ax-fault-sub', key: 's' }, 'unmapped · bus fault'),
      ]),
    ]),
    // disabled shelf
    h('div', { className: 'ax-disabled', key: 'dis' }, [
      h('button', { className: 'ax-disabled-hd', key: 'hd', onClick: (e) => { e.stopPropagation(); setDisOpen(o => !o); } }, [
        h('span', { className: 'ax-disabled-chev' + (disOpen ? ' open' : ''), key: 'c' }, UI.chevron({ s: 12 })),
        h('span', { key: 't' }, [h('b', { key: 'b' }, window.AX_DISABLED_TOTAL), ' declared-but-disabled peripherals']),
      ]),
      disOpen && h('div', { className: 'ax-disabled-list', key: 'list' }, [
        ...window.AX_DISABLED.map(d => h('span', { className: 'ax-dis-chip', key: d }, d)),
        h('span', { className: 'ax-dis-more', key: 'more' }, '+' + (window.AX_DISABLED_TOTAL - window.AX_DISABLED.length) + ' more'),
      ]),
    ]),
    // foot
    h('div', { className: 'ax-map-foot', key: 'foot' }, [
      h('div', { className: 'ax-legend', key: 'leg' }, [
        h('span', { className: 'lg', key: '1' }, [h('i', { className: 'lg-dot active', key: 'd' }), 'active']),
        h('span', { className: 'lg', key: '2' }, [h('i', { className: 'lg-dot bus', key: 'd' }), 'open bus port']),
        crashed && h('span', { className: 'lg', key: '3' }, [h('i', { className: 'lg-dot fault', key: 'd' }), 'fault']),
      ]),
      !crashed && counter != null && h('span', { className: 'ax-flow-note', key: 'fn' }, '● axyr_counter = ' + counter.toLocaleString('en-US') + ' → SRAM'),
    ]),
  ]);
}

window.SystemMap = SystemMap;
