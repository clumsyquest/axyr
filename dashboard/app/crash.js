// ============================================================
// Axyr Cockpit — Crash panel. Takes over when the core faults.
// Verbatim from snapshot.crash (crash_demo): cause, location,
// call stack, registers, recent serial telemetry. + actions.
// ============================================================
const hc = React.createElement;

function Crash({ onOpenFile, onAskAgent, onReboot }) {
  const c = window.AX_CRASH;
  const loc = c.location;

  return hc('div', { className: 'ax-card crash grow', key: 'crash' }, [
    hc('div', { className: 'ax-crash-head', key: 'h' }, [
      hc('span', { className: 'ax-crash-badge', key: 'b' }, [UI.warn({ s: 14 }), 'CRASH']),
      hc('span', { className: 'ax-crash-kind', key: 'k' }, 'Bus fault · precise · reason ' + c.reason_code),
      hc('span', { className: 'ax-card-sub', key: 's' }, 'get_last_crash'),
    ]),
    hc('div', { className: 'ax-card-body', key: 'body' }, [
      // plain cause
      hc('p', { className: 'ax-crash-cause', key: 'cause' }, c.cause),

      // where
      hc('div', { className: 'ax-sub-label', key: 'wl' }, 'Where it died'),
      hc('button', {
        className: 'ax-crash-where', key: 'where',
        onClick: () => onOpenFile(loc.file, loc.line),
      }, [
        hc('span', { className: 'ax-crash-fn mono', key: 'fn' }, loc.function + '()'),
        hc('span', { className: 'ax-crash-file mono', key: 'f' }, loc.file.replace('firmware/', '') + ':' + loc.line),
        hc('span', { className: 'ax-crash-open', key: 'o' }, 'open ↗'),
      ]),
      hc('div', { className: 'ax-crash-fault mono', key: 'fa' }, 'fault address  ' + c.fault_address + '  ·  unmapped on STM32F401'),

      // call stack
      hc('div', { className: 'ax-sub-label', key: 'csl' }, 'Call stack'),
      hc('div', { className: 'ax-stack', key: 'stack' }, c.call_stack.map((f) =>
        hc('button', { className: 'ax-frame' + (f.frame === 0 ? ' culprit' : ''), key: f.frame,
          onClick: () => onOpenFile(f.file, f.line) }, [
          hc('span', { className: 'ax-frame-idx mono', key: 'i' }, '#' + f.frame),
          hc('span', { className: 'ax-frame-fn mono', key: 'fn' }, f.function),
          hc('span', { className: 'ax-frame-loc mono', key: 'l' }, f.file.split('/').pop() + ':' + f.line),
        ])
      )),

      // registers
      hc('div', { className: 'ax-sub-label', key: 'rl' }, 'Registers at fault'),
      hc('div', { className: 'ax-crash-regs', key: 'regs' }, Object.entries(c.registers).map(([k, v]) =>
        hc('div', { className: 'ax-creg', key: k }, [
          hc('span', { className: 'ax-creg-k mono', key: 'k' }, k.toUpperCase()),
          hc('span', { className: 'ax-creg-v mono', key: 'v' }, v),
        ])
      )),

      // recent telemetry
      hc('div', { className: 'ax-sub-label', key: 'tl' }, 'Recent serial · leading up to the fault'),
      hc('div', { className: 'ax-serial', key: 'serial' }, c.recent_telemetry.map((line, i) => {
        const err = line.includes('<err>') || line.includes('FAULT');
        return hc('div', { className: 'ax-serial-line' + (err ? ' err' : ''), key: i, }, line);
      })),
    ]),
    // actions footer
    hc('div', { className: 'ax-crash-actions', key: 'act' }, [
      hc('button', { className: 'ax-btn ghost', key: 'r', onClick: onReboot }, [UI.reboot({ s: 15 }), 'reboot_board']),
      hc('button', { className: 'ax-btn primary', key: 'a', onClick: onAskAgent }, [UI.agent({ s: 15 }), 'Hand to agent']),
    ]),
  ]);
}

window.Crash = Crash;
