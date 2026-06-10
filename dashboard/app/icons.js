// ============================================================
// Axyr Cockpit — kind icons (abstract, geometric) + UI glyphs.
// A small icon set we provide, one per devicetree `kind`.
// All use currentColor + stroke; theme-agnostic. No photorealism.
// ============================================================
const { createElement: hh } = React;

// generic svg wrapper
function Svg({ s = 18, children, sw = 1.6, vb = 24, style, key }) {
  const kids = Array.isArray(children) ? children : [children];
  return hh('svg', { key: key || 'icon', width: s, height: s, viewBox: `0 0 ${vb} ${vb}`, fill: 'none',
    stroke: 'currentColor', strokeWidth: sw, strokeLinecap: 'round', strokeLinejoin: 'round', style }, ...kids);
}
const P = (d, extra) => hh('path', { d, ...(extra || {}) });
const L = (x1,y1,x2,y2) => hh('line', { x1, y1, x2, y2 });
const C = (cx,cy,r,extra) => hh('circle', { cx, cy, r, ...(extra||{}) });
const R = (x,y,w,h,rx) => hh('rect', { x, y, width:w, height:h, rx: rx==null?2:rx });

// one drawing per kind — simple primitives only
const KIND_ICONS = {
  soc:     (p) => Svg({ ...p, children: [R(5,5,14,14,2), R(9,9,6,6,1)] }),
  core:    (p) => Svg({ ...p, children: [R(6,6,12,12,2), L(3,9,6,9), L(3,12,6,12), L(3,15,6,15), L(18,9,21,9), L(18,12,21,12), L(18,15,21,15), L(9,3,9,6), L(12,3,12,6), L(15,3,15,6), L(9,18,9,21), L(12,18,12,21), L(15,18,15,21)] }),
  ram:     (p) => Svg({ ...p, children: [R(3,7,18,10,2), L(7,17,7,20), L(12,17,12,20), L(17,17,17,20), L(7,7,7,12), L(12,7,12,12), L(17,7,17,12)] }),
  clock:   (p) => Svg({ ...p, children: [C(12,12,8), P('M12 8 L12 12 L15 14')] }),
  sensor:  (p) => Svg({ ...p, children: [C(12,12,3.2), C(12,12,8, { strokeDasharray:'2 3' }), L(12,1,12,3), L(12,21,12,23)] }),
  led:     (p) => Svg({ ...p, children: [C(12,11,5), L(8.5,16,15.5,16), L(9.5,18.5,14.5,18.5), L(12,2,12,4.5), P('M5 5 L6.6 6.6'), P('M19 5 L17.4 6.6')] }),
  button:  (p) => Svg({ ...p, children: [R(4,10,16,7,3.5), C(8,13.5,1.4), C(12,13.5,1.4), C(16,13.5,1.4)] }),
  i2c:     (p) => Svg({ ...p, children: [L(4,8,20,8), L(4,16,20,16), C(9,8,1.6), C(15,16,1.6)] }),
  spi:     (p) => Svg({ ...p, children: [L(4,7,20,7), L(4,12,20,12), L(4,17,20,17), C(8,7,1.3), C(13,12,1.3), C(18,17,1.3)] }),
  uart:    (p) => Svg({ ...p, children: [P('M3 8 L10 8 L13 12 L21 12'), P('M3 16 L8 16 L11 12'), C(20,12,1.2)] }),
  adc:     (p) => Svg({ ...p, children: [P('M3 16 C 7 16, 7 8, 11 8 C 15 8, 15 16, 19 16'), L(19,16,21,16), R(3,4,3,3,0.5), R(8,4,3,3,0.5)] }),
  timer:   (p) => Svg({ ...p, children: [C(12,13,7), P('M9.5 4 L14.5 4'), L(12,4,12,6), P('M12 13 L12 9'), P('M12 13 L15 13')] }),
  pwm:     (p) => Svg({ ...p, children: [P('M3 8 L7 8 L7 16 L11 16 L11 8 L15 8 L15 16 L19 16 L19 8 L21 8')] }),
  rtc:     (p) => Svg({ ...p, children: [C(12,12,8), P('M12 8 L12 12 L15 13'), L(12,4,12,2)] }),
  watchdog:(p) => Svg({ ...p, children: [C(12,12,7), C(12,12,2), P('M12 5 L12 2 M19 12 L22 12 M12 19 L12 22 M5 12 L2 12'), P('M12 12 L16 9') ] }),
  irq:     (p) => Svg({ ...p, children: [P('M13 2 L4 14 L11 14 L9 22 L20 9 L13 9 Z')] }),
  flash:   (p) => Svg({ ...p, children: [R(4,4,16,16,2), L(4,9,20,9), L(4,14,20,14), C(7,6.5,0.8), C(7,11.5,0.8), C(7,16.5,0.8)] }),
  usb:     (p) => Svg({ ...p, children: [C(6,18,2), P('M6 16 L6 6 L10 6'), P('M6 11 L13 11 L13 7 L16 9 L13 11'), R(15,3,5,4,1)] }),
  header:  (p) => Svg({ ...p, children: [R(3,5,18,14,2), C(7,9,0.9), C(11,9,0.9), C(15,9,0.9), C(7,14,0.9), C(11,14,0.9), C(15,14,0.9)] }),
  gpio:    (p) => Svg({ ...p, children: [R(5,5,14,14,2), L(2,9,5,9), L(2,15,5,15), L(19,9,22,9), L(19,15,22,15)] }),
};
function kindIcon(kind, props) {
  const fn = KIND_ICONS[kind] || KIND_ICONS.gpio;
  return fn(props || {});
}
const KIND_LABEL = {
  soc:'system-on-chip', core:'CPU core', ram:'memory', clock:'clock', sensor:'sensor',
  led:'LED', button:'button', i2c:'I²C bus', spi:'SPI bus', uart:'UART', adc:'ADC',
  timer:'timer', pwm:'PWM', rtc:'real-time clock', watchdog:'watchdog', irq:'interrupts',
  flash:'flash', usb:'USB PHY', header:'connector', gpio:'GPIO',
};

// ---- small UI glyphs ---------------------------------------
const UI = {
  map:      (p)=>Svg({...p, sw:1.5, children:[P('M9 3 L3 6 L3 21 L9 18 L15 21 L21 18 L21 3 L15 6 Z'), L(9,3,9,18), L(15,6,15,21)]}),
  agent:    (p)=>Svg({...p, sw:1.5, children:[R(5,7,14,12,3), L(12,3,12,7), C(12,3,1), C(9.5,12.5,1), C(14.5,12.5,1), L(9,16,15,16), L(2,11,5,11), L(19,11,22,11)]}),
  reboot:   (p)=>Svg({...p, sw:1.5, children:[P('M21 12 a9 9 0 1 1 -2.6 -6.3'), P('M21 3 L21 8 L16 8')]}),
  flash:    (p)=>Svg({...p, sw:1.5, children:[P('M13 2 L4 14 L11 14 L9 22 L20 9 L13 9 Z')]}),
  pulse:    (p)=>Svg({...p, sw:1.5, children:[P('M2 12 L7 12 L10 5 L14 19 L17 12 L22 12')]}),
  chip:     (p)=>Svg({...p, sw:1.5, children:[R(7,7,10,10,1.5)]}),
  send:     (p)=>Svg({...p, sw:1.5, children:[P('M4 12 L20 4 L13 20 L11 13 Z')]}),
  file:     (p)=>Svg({...p, sw:1.5, children:[P('M6 3 L14 3 L19 8 L19 21 L6 21 Z'), P('M14 3 L14 8 L19 8')]}),
  folder:   (p)=>Svg({...p, sw:1.5, children:[P('M3 6 L9 6 L11 9 L21 9 L21 19 L3 19 Z')]}),
  chevron:  (p)=>Svg({...p, sw:1.6, children:[P('M9 6 L15 12 L9 18')]}),
  dot:      (p)=>Svg({...p, children:[C(12,12,4,{fill:'currentColor',stroke:'none'})]}),
  warn:     (p)=>Svg({...p, sw:1.6, children:[P('M12 3 L22 20 L2 20 Z'), L(12,9,12,14), L(12,17,12,17.2)]}),
  check:    (p)=>Svg({...p, sw:1.8, children:[P('M4 12 L10 18 L20 5')]}),
  close:    (p)=>Svg({...p, sw:1.6, children:[L(6,6,18,18), L(18,6,6,18)]}),
  search:   (p)=>Svg({...p, sw:1.6, children:[C(11,11,7), L(16,16,21,21)]}),
  scrub:    (p)=>Svg({...p, sw:1.5, children:[L(3,12,21,12), C(8,12,2,{fill:'currentColor',stroke:'none'})]}),
};

Object.assign(window, { AxSvg: Svg, kindIcon, KIND_LABEL, UI });
