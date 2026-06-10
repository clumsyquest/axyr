// ============================================================
// Axyr Cockpit — Agent workspace (Claude-Code style).
// [ file tree | code viewer | chat ]. The agent reads the real
// board state through the real MCP tools and can act on it.
// ============================================================
const ha = React.createElement;
const { useState: uS, useEffect: uE, useRef: uR } = React;

// ---- tiny C highlighter ------------------------------------
function hlC(src) {
  let s = src.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
  const re = /(\/\*[\s\S]*?\*\/|\/\/[^\n]*)|("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*')|(#\s*\w+)|\b(0x[0-9A-Fa-f]+|\d+\.?\d*)\b|\b(uint32_t|uint8_t|uint16_t|int16_t|int32_t|size_t|bool|void|int|char|unsigned)\b|\b(static|volatile|const|struct|return|while|for|if|else|sizeof|__attribute__)\b/g;
  return s.replace(re, (m, com, str, pre, num, ty, kw) => {
    if (com) return '<span class="c-com">' + com + '</span>';
    if (str) return '<span class="c-str">' + str + '</span>';
    if (pre) return '<span class="c-pre">' + pre + '</span>';
    if (num) return '<span class="c-num">' + num + '</span>';
    if (ty)  return '<span class="c-ty">' + ty + '</span>';
    if (kw)  return '<span class="c-kw">' + kw + '</span>';
    return m;
  });
}

// ---- file tree ---------------------------------------------
function TreeNode({ node, depth, openPath, onOpen }) {
  const [open, setOpen] = uS(node.open !== false);
  if (node.type === 'dir') {
    return ha('div', { className: 'ax-tn-dir' }, [
      ha('button', { className: 'ax-tn-row', key: 'r', style: { paddingLeft: 8 + depth * 12 }, onClick: () => setOpen(o => !o) }, [
        ha('span', { className: 'ax-tn-chev' + (open ? ' open' : ''), key: 'c' }, UI.chevron({ s: 12 })),
        ha('span', { className: 'ax-tn-ic', key: 'i' }, UI.folder({ s: 14 })),
        ha('span', { className: 'ax-tn-name', key: 'n' }, node.name),
      ]),
      open && ha('div', { key: 'ch' }, node.children.map((c, i) =>
        ha(TreeNode, { node: c, depth: depth + 1, openPath, onOpen, key: i }))),
    ]);
  }
  const active = openPath === node.path;
  return ha('button', {
    className: 'ax-tn-row file' + (active ? ' active' : ''), style: { paddingLeft: 8 + depth * 12 },
    onClick: () => onOpen(node.path),
  }, [
    ha('span', { className: 'ax-tn-ic', key: 'i' }, UI.file({ s: 14 })),
    ha('span', { className: 'ax-tn-name', key: 'n' }, node.name),
    node.crash && ha('span', { className: 'ax-tn-badge', key: 'b' }, 'fault'),
  ]);
}

// ---- code viewer -------------------------------------------
function CodeView({ path, activeLine }) {
  const file = window.AX_FILES[path];
  const scRef = uR(null);
  const LH = 21;
  uE(() => {
    if (activeLine && scRef.current) {
      const top = Math.max(0, (activeLine - 6) * LH);
      scRef.current.scrollTop = top;
    }
  }, [path, activeLine]);
  if (!file) return ha('div', { className: 'ax-code empty' }, 'select a file');
  const lines = file.body.replace(/\n$/, '').split('\n');
  return ha('div', { className: 'ax-code' }, [
    ha('div', { className: 'ax-code-tab', key: 'tab' }, [
      ha('span', { className: 'ax-code-ic', key: 'i' }, UI.file({ s: 13 })),
      ha('span', { className: 'ax-code-path mono', key: 'p' }, path),
    ]),
    ha('div', { className: 'ax-code-scroll', key: 'sc', ref: scRef }, [
      activeLine && ha('div', { className: 'ax-code-band', key: 'band', style: { top: (activeLine - 1) * LH + 'px', height: LH + 'px' } }),
      ha('div', { className: 'ax-code-grid', key: 'g' }, [
        ha('div', { className: 'ax-gutter mono', key: 'gut' }, lines.map((_, i) =>
          ha('div', { className: 'ax-ln' + (i + 1 === activeLine ? ' hot' : ''), key: i }, [
            i + 1 === activeLine && ha('span', { className: 'ax-ln-dot', key: 'd' }),
            String(i + 1),
          ]))),
        ha('pre', { className: 'ax-pre mono', key: 'pre', dangerouslySetInnerHTML: { __html: hlC(file.body.replace(/\n$/, '')) } }),
      ]),
    ]),
  ]);
}

// ---- chat: scripted, but on REAL tools + REAL data ---------
const J = (o) => JSON.stringify(o);
function scenario(intent, onFlash, onReboot) {
  const C = window.AX_CRASH;
  if (intent === 'why') return [
    { role: 'tool', name: 'get_last_crash', args: '', result: 'cause: "' + C.cause + '"\nlocation: ' + C.location.function + ' @ ' + C.location.file.replace('firmware/', '') + ':' + C.location.line + '\nfault_address: ' + C.fault_address + '\nframes: i2c_read_reg → read_sensor → main' },
    { role: 'agent', text: 'The core took a **precise bus fault**. In `i2c_read_reg()` (crash_demo/src/main.c:30) the firmware dereferences `0xBADCAFE0` — an address that isn\'t mapped on the STM32F401 — so the read raised the fault. The path was `main()` → `read_sensor()` → `i2c_read_reg()`.' },
    { role: 'agent', text: 'Note: this is **crash_demo**, which faults on purpose to exercise capture. The pointer `(volatile uint32_t *)(0xBADCAFE0 + reg)` is the deliberate bad access. Want me to propose a guarded version, or flash `live_demo` instead?' },
  ];
  if (intent === 'fix') return [
    { role: 'tool', name: 'read_memory', args: J({ address: '0xbadcafe0', count: 4 }), result: 'BusFault — region unmapped (no readable memory at 0xbadcafe0)' },
    { role: 'agent', text: 'For real firmware the fix is to never read an unmapped address — validate the pointer / use the peripheral\'s real base. Here\'s the change on `i2c_read_reg`:' },
    { role: 'diff', file: 'firmware/crash_demo/src/main.c', minus: 'volatile uint32_t *bad_ptr = (volatile uint32_t *)(0xBADCAFE0 + reg);', plus: 'volatile uint32_t *reg_ptr = (volatile uint32_t *)(I2C1_BASE + reg);' },
    { role: 'agent', text: 'I can apply it and rebuild, or — since crash_demo is meant to crash — flash `live_demo` to get a healthy running board.' },
  ];
  if (intent === 'flash') return [
    { role: 'tool', name: 'flash_firmware', args: J({ path: 'build/live_demo/zephyr/zephyr.elf' }), result: 'erase ✓  write 18.4 KB ✓  verify ✓  reset ✓  core: running' },
    { role: 'agent', text: 'Flashed **live_demo** and reset the board. `axyr_counter` is now incrementing every 250 ms — open **System** to watch it live.', act: 'flashed' },
  ];
  if (intent === 'reboot') return [
    { role: 'tool', name: 'reboot_board', args: '', result: 'NVIC SystemReset asserted · core: running' },
    { role: 'agent', text: 'Board rebooted. It\'s running again — though crash_demo will fault on its next loop. Flash live_demo if you want it to stay up.', act: 'rebooted' },
  ];
  return [{ role: 'agent', text: 'I read this board live over the debug probe (the same MCP tools, but in-app). Ask me why it crashed, to propose a fix, or to flash / reboot.' }];
}

function ToolCard({ m }) {
  return ha('div', { className: 'ax-msg-tool' }, [
    ha('div', { className: 'ax-tool-line', key: 'l' }, [
      ha('span', { className: 'ax-tool-tag', key: 't' }, 'tool'),
      ha('span', { className: 'ax-tool-call mono', key: 'c' }, m.name + '(' + (m.args || '') + ')'),
    ]),
    ha('pre', { className: 'ax-tool-res mono', key: 'r' }, m.result),
  ]);
}
function DiffCard({ m }) {
  return ha('div', { className: 'ax-diff' }, [
    ha('div', { className: 'ax-diff-file mono', key: 'f' }, m.file.replace('firmware/', '')),
    ha('div', { className: 'ax-diff-row minus mono', key: 'm' }, [ha('span', { className: 'ax-diff-sign', key: 's' }, '-'), m.minus]),
    ha('div', { className: 'ax-diff-row plus mono', key: 'p' }, [ha('span', { className: 'ax-diff-sign', key: 's' }, '+'), m.plus]),
  ]);
}
function fmtText(t) {
  // **bold** and `code`
  const parts = [];
  const re = /\*\*([^*]+)\*\*|`([^`]+)`/g; let last = 0, m, i = 0;
  while ((m = re.exec(t))) {
    if (m.index > last) parts.push(t.slice(last, m.index));
    if (m[1]) parts.push(ha('b', { key: i++ }, m[1]));
    else parts.push(ha('code', { key: i++ }, m[2]));
    last = re.lastIndex;
  }
  if (last < t.length) parts.push(t.slice(last));
  return parts;
}

function Chat({ mode, kick, onOpenFile, onFlash, onReboot }) {
  const [msgs, setMsgs] = uS([{ role: 'agent', text: 'Connected to **' + window.AX_DEVICE.board + '** over ' + window.AX_DEVICE.probe + '. I read and act on this board live — ask away, or hand me a crash.' }]);
  const [busy, setBusy] = uS(false);
  const [input, setInput] = uS('');
  const endRef = uR(null);
  const lastKick = uR(0);

  const push = (arr) => {
    let i = 0;
    const step = () => {
      if (i >= arr.length) { setBusy(false); return; }
      const item = arr[i]; i++;
      setMsgs(m => [...m, item]);
      if (item.act === 'flashed') onFlash && onFlash();
      if (item.act === 'rebooted') onReboot && onReboot();
      setTimeout(step, item.role === 'tool' ? 560 : 720);
    };
    setBusy(true);
    setTimeout(step, 360);
  };
  const run = (intent, userText) => {
    if (userText) setMsgs(m => [...m, { role: 'user', text: userText }]);
    push(scenario(intent, onFlash, onReboot));
  };

  uE(() => {
    if (kick && kick !== lastKick.current) {
      lastKick.current = kick;
      run('why', 'Why did the board crash?');
    }
  }, [kick]);

  uE(() => { if (endRef.current) endRef.current.scrollTop = endRef.current.scrollHeight; }, [msgs, busy]);

  const prompts = mode === 'crashed'
    ? [['why', 'Why did it crash?'], ['fix', 'Propose a fix'], ['flash', 'Flash live_demo'], ['reboot', 'Reboot']]
    : [['why', 'Explain last crash'], ['fix', 'Propose a fix'], ['flash', 'Flash live_demo'], ['reboot', 'Reboot board']];

  const submit = () => {
    const t = input.trim(); if (!t || busy) return;
    setInput('');
    const low = t.toLowerCase();
    let intent = 'generic';
    if (/crash|fault|why|halt/.test(low)) intent = 'why';
    else if (/fix|patch|repair|guard/.test(low)) intent = 'fix';
    else if (/flash|live/.test(low)) intent = 'flash';
    else if (/reboot|reset|restart/.test(low)) intent = 'reboot';
    run(intent, t);
  };

  return ha('div', { className: 'ax-chat' }, [
    ha('div', { className: 'ax-chat-head', key: 'h' }, [
      ha('span', { className: 'ax-chat-ic', key: 'i' }, UI.agent({ s: 16 })),
      ha('div', { key: 'tx' }, [
        ha('div', { className: 'ax-chat-title', key: 't' }, 'Axyr agent'),
        ha('div', { className: 'ax-chat-sub', key: 's' }, 'in-app · reads & acts on the board'),
      ]),
      ha('span', { className: 'ax-chat-live', key: 'l' }, [ha('i', { key: 'd' }), mode === 'crashed' ? 'halted' : 'live']),
    ]),
    ha('div', { className: 'ax-chat-scroll', key: 'sc', ref: endRef }, [
      ...msgs.map((m, i) => {
        if (m.role === 'tool') return ha(ToolCard, { m, key: i });
        if (m.role === 'diff') return ha(DiffCard, { m, key: i });
        const isUser = m.role === 'user';
        return ha('div', { className: 'ax-msg ' + (isUser ? 'user' : 'agent'), key: i }, [
          !isUser && ha('span', { className: 'ax-msg-av', key: 'a' }, UI.agent({ s: 13 })),
          ha('div', { className: 'ax-msg-bub', key: 'b' }, fmtText(m.text)),
        ]);
      }),
      busy && ha('div', { className: 'ax-msg agent', key: 'busy' }, [
        ha('span', { className: 'ax-msg-av', key: 'a' }, UI.agent({ s: 13 })),
        ha('div', { className: 'ax-msg-bub typing', key: 'b' }, [ha('i', { key: 1 }), ha('i', { key: 2 }), ha('i', { key: 3 })]),
      ]),
    ]),
    ha('div', { className: 'ax-chat-prompts', key: 'pr' }, prompts.map(([intent, label], i) =>
      ha('button', { className: 'ax-prompt', key: i, disabled: busy, onClick: () => run(intent, label) }, label))),
    ha('div', { className: 'ax-chat-input', key: 'in' }, [
      ha('input', { key: 'i', value: input, placeholder: 'Ask the agent…', disabled: busy,
        onChange: (e) => setInput(e.target.value), onKeyDown: (e) => { if (e.key === 'Enter') submit(); } }),
      ha('button', { key: 's', className: 'ax-send', disabled: busy || !input.trim(), onClick: submit }, UI.send({ s: 16 })),
    ]),
  ]);
}

function AgentView({ mode, openPath, setOpenPath, activeLine, setActiveLine, kick, onFlash, onReboot }) {
  const openFile = (path, line) => { setOpenPath(path); setActiveLine(line || null); };
  return ha('div', { className: 'ax-agent' }, [
    ha('div', { className: 'ax-tree', key: 'tree' }, [
      ha('div', { className: 'ax-tree-head', key: 'h' }, 'PROJECT · crash_demo'),
      ha('div', { className: 'ax-tree-body', key: 'b' }, window.AX_FILE_TREE.map((n, i) =>
        ha(TreeNode, { node: n, depth: 0, openPath, onOpen: (p) => { setOpenPath(p); setActiveLine(null); }, key: i }))),
    ]),
    ha(CodeView, { path: openPath, activeLine, key: 'code' }),
    ha(Chat, { mode, kick, onOpenFile: openFile, onFlash, onReboot, key: 'chat' }),
  ]);
}

window.AgentView = AgentView;
