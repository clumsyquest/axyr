// ============================================================
// Axyr Cockpit — REAL data only.
// Sourced verbatim from the repo: docs/system-snapshot.example.json,
// the devicetree graph (get_system_map), and firmware/*/src/main.c.
// NOTHING here is invented. Two states match the two real demo
// firmwares: live_demo (RUNNING) and crash_demo (CRASHED).
// ============================================================

// ---- Device identity (snapshot.device) ---------------------
const DEVICE = {
  board: 'STM32F401RE-NUCLEO',
  vendor: 'STMicroelectronics',
  chip: 'STM32F401RETx',
  core: 'ARM Cortex-M4F',
  cpuid: '0x410fc241',
  probe: 'ST-LINK/V2.1',
  transport: 'RTT',
  clkMHz: 84,
};

// ---- System map graph (from the Zephyr devicetree) ---------
// kind drives the icon. tier drives the layout ring.
// status okay|disabled. address is the DT reg base (hex) or null.
// Buses are empty in the demo firmwares — no external sensor is
// declared — which is the truth; the structure is ready for one.
const NODES = [
  // the hub
  { id: 'soc',   label: 'soc',        kind: 'soc',     tier: 'hub',   addr: null,         compatible: 'st,stm32f401', sub: 'STM32F401' },
  // board level (siblings of soc in the DT, rooted under board)
  { id: 'cpu0',  label: 'cpu0',       kind: 'core',    tier: 'board', addr: null,         compatible: 'arm,cortex-m4f' },
  { id: 'sram0', label: 'sram0',      kind: 'ram',     tier: 'board', addr: '0x20000000', compatible: 'mmio-sram', sub: '96 KB' },
  { id: 'pll',   label: 'clk · pll',  kind: 'clock',   tier: 'board', addr: null,         compatible: 'st,stm32f4-pll-clock', sub: 'HSE · 84 MHz' },
  { id: 'die_temp', label: 'die_temp', kind: 'sensor', tier: 'board', addr: null,         compatible: 'st,stm32-temp-cal' },
  { id: 'leds',  label: 'leds',       kind: 'led',     tier: 'board', addr: null,         compatible: 'gpio-leds' },
  { id: 'pwmleds', label: 'pwmleds',  kind: 'led',     tier: 'board', addr: null,         compatible: 'pwm-leds' },
  { id: 'gpio_keys', label: 'gpio_keys', kind: 'button', tier: 'board', addr: null,       compatible: 'gpio-keys' },
  { id: 'arduino_header', label: 'arduino_header', kind: 'header', tier: 'board', addr: null, compatible: 'arduino-header-r3' },
  { id: 'otgfs', label: 'otgfs_phy',  kind: 'usb',     tier: 'board', addr: null,         compatible: 'usb-nop-xceiv' },
  // soc children (peripherals on the hub)
  { id: 'rcc',   label: 'rcc',        kind: 'clock',   tier: 'soc',   addr: '0x40023800', compatible: 'st,stm32-rcc' },
  { id: 'flash', label: 'flash',      kind: 'flash',   tier: 'soc',   addr: '0x40023c00', compatible: 'st,stm32-flash-controller', sub: '512 KB' },
  { id: 'usart1', label: 'usart1',    kind: 'uart',    tier: 'soc',   addr: '0x40011000', compatible: 'st,stm32-usart' },
  { id: 'usart2', label: 'usart2',    kind: 'uart',    tier: 'soc',   addr: '0x40004400', compatible: 'st,stm32-usart' },
  { id: 'i2c1',  label: 'i2c1 · arduino_i2c', kind: 'i2c', tier: 'soc', addr: '0x40005400', compatible: 'st,stm32-i2c-v1', bus: true },
  { id: 'i2c3',  label: 'i2c3',       kind: 'i2c',     tier: 'soc',   addr: '0x40005c00', compatible: 'st,stm32-i2c-v1', bus: true },
  { id: 'spi1',  label: 'spi1 · arduino_spi', kind: 'spi', tier: 'soc', addr: '0x40013000', compatible: 'st,stm32-spi', bus: true },
  { id: 'spi2',  label: 'spi2',       kind: 'spi',     tier: 'soc',   addr: '0x40003800', compatible: 'st,stm32-spi', bus: true },
  { id: 'timers2', label: 'timers2',  kind: 'timer',   tier: 'soc',   addr: '0x40000000', compatible: 'st,stm32-timers' },
  { id: 'pwm2',  label: 'pwm2',       kind: 'pwm',     tier: 'leaf',  addr: null,         compatible: 'st,stm32-pwm', parent: 'timers2' },
  { id: 'adc1',  label: 'adc1',       kind: 'adc',     tier: 'soc',   addr: '0x40012000', compatible: 'st,stm32f4-adc' },
  { id: 'rtc',   label: 'rtc',        kind: 'rtc',     tier: 'soc',   addr: '0x40002800', compatible: 'st,stm32-rtc' },
  { id: 'wwdg',  label: 'wwdg',       kind: 'watchdog',tier: 'soc',   addr: '0x40002c00', compatible: 'st,stm32-window-watchdog' },
  { id: 'exti',  label: 'exti',       kind: 'irq',     tier: 'soc',   addr: '0x40013c00', compatible: 'st,stm32-exti' },
];

// edges: board -> board-nodes + soc ; soc -> soc-children ; timers2 -> pwm2
const BOARD_CHILDREN = ['cpu0','sram0','pll','die_temp','leds','pwmleds','gpio_keys','arduino_header','otgfs'];
const SOC_CHILDREN = ['rcc','flash','usart1','usart2','i2c1','i2c3','spi1','spi2','timers2','adc1','rtc','wwdg','exti'];

// declared-but-disabled (greyed shelf). Real DT disables; representative subset.
const DISABLED = ['i2c2','spi3','spi4','usart6','timers1','timers3','timers4','timers5','dma1','dma2','can1','sdio'];
const DISABLED_TOTAL = 31;

// ---- Threads (snapshot.threads) ----------------------------
const THREADS = [
  { name: 'main',            stack_used: 320, stack_total: 1024, stack_pct: 31, cpu_pct: 0 },
  { name: 'idle',            stack_used: 64,  stack_total: 320,  stack_pct: 20, cpu_pct: 99 },
  { name: 'thread_analyzer', stack_used: 552, stack_total: 1024, stack_pct: 53, cpu_pct: 0 },
];

// ---- Timeline / context switches (snapshot.timeline) -------
const TIMELINE = [
  { cycles: 0,        thread: 'main' },
  { cycles: 3984,     thread: 'idle' },
  { cycles: 21008400, thread: 'main' },
  { cycles: 21012384, thread: 'idle' },
];

// ---- Watched variable (snapshot.variables) -----------------
// live_demo increments axyr_counter every 250 ms; we replay that.
const VAR_BASE = { name: 'axyr_counter', address: '0x200008a0', type: 'uint32' };

// ---- Decoded peripheral (snapshot.peripherals) -------------
// Only RCC is captured in the contract example. We show exactly that;
// other peripherals are inspectable on demand via read_peripheral.
const PERIPHERAL_RCC = {
  name: 'RCC', address: '0x40023800',
  registers: [
    { name: 'CR', value: '0x03077483', fields: [
      { name: 'PLLRDY', value: 1, meaning: 'PLL locked' },
      { name: 'PLLON',  value: 1, meaning: 'PLL enabled' },
      { name: 'HSEON',  value: 1, meaning: 'HSE oscillator on' },
    ]},
    { name: 'APB1ENR', value: '0x10020000', fields: [
      { name: 'USART2EN', value: 1, meaning: 'USART2 clock enabled' },
      { name: 'PWREN',    value: 1, meaning: 'Power interface clock enabled' },
    ]},
  ],
};

// ---- Crash post-mortem (snapshot.crash) --------------------
const CRASH = {
  cause: 'Bus fault: invalid memory access (precise)',
  reason_code: 25,
  fault_address: '0xbadcafe0',
  location: { function: 'i2c_read_reg', file: 'firmware/crash_demo/src/main.c', line: 30 },
  call_stack: [
    { frame: 0, function: 'i2c_read_reg', file: 'firmware/crash_demo/src/main.c', line: 30 },
    { frame: 1, function: 'read_sensor',  file: 'firmware/crash_demo/src/main.c', line: 36 },
    { frame: 2, function: 'main',         file: 'firmware/crash_demo/src/main.c', line: 43 },
  ],
  registers: { pc: '0x0800046a', lr: '0x080004b5', xpsr: '0x61000000', r3: '0xbadca000' },
  recent_telemetry: [
    '*** Booting Zephyr OS build v4.4.0 ***',
    'About to crash...',
    '<err> os: ***** BUS FAULT *****',
    '<err> os:   Precise data bus error',
    '<err> os:   BFAR Address: 0xbadcafe0',
  ],
};

// ---- MCP actions / tools (snapshot.actions) ----------------
const ACTIONS = [
  { name: 'get_last_crash',  kind: 'read' },
  { name: 'get_system_map',  kind: 'read' },
  { name: 'get_threads',     kind: 'read' },
  { name: 'get_trace',       kind: 'read' },
  { name: 'read_variable',   kind: 'read',   args: ['name'] },
  { name: 'read_peripheral', kind: 'read',   args: ['name'] },
  { name: 'read_memory',     kind: 'read',   args: ['address', 'count'] },
  { name: 'reboot_board',    kind: 'action' },
  { name: 'flash_firmware',  kind: 'action', args: ['path'] },
];

// ---- State summaries per mode ------------------------------
const STATE = {
  running: {
    core: 'running',
    summary: 'live_demo running — axyr_counter incrementing every 250 ms',
    reset_reason: 'power-on reset (RCC CSR: PORRSTF)',
    clocks: 'PLL on/locked · HSE on · SYSCLK from PLL · 84 MHz',
    firmware: 'crash_demo  →  live_demo',
  },
  crashed: {
    core: 'crashed',
    summary: 'Bus fault: invalid memory access (precise)',
    reset_reason: 'software reset (RCC CSR: SFTRSTF)',
    clocks: 'PLL on/locked · HSE on · SYSCLK from PLL · 84 MHz',
    firmware: 'crash_demo',
  },
};

// nodes considered "active" in the live (running) board.
// live_demo: CPU runs, writes axyr_counter to SRAM, ticks systick,
// streams printk over RTT. Buses idle (no external device declared).
const ACTIVE_RUNNING = ['cpu0', 'sram0', 'rcc', 'pll', 'flash'];

Object.assign(window, {
  AX_DEVICE: DEVICE,
  AX_NODES: NODES,
  AX_BOARD_CHILDREN: BOARD_CHILDREN,
  AX_SOC_CHILDREN: SOC_CHILDREN,
  AX_DISABLED: DISABLED,
  AX_DISABLED_TOTAL: DISABLED_TOTAL,
  AX_THREADS: THREADS,
  AX_TIMELINE: TIMELINE,
  AX_VAR_BASE: VAR_BASE,
  AX_RCC: PERIPHERAL_RCC,
  AX_CRASH: CRASH,
  AX_ACTIONS: ACTIONS,
  AX_STATE: STATE,
  AX_ACTIVE_RUNNING: ACTIVE_RUNNING,
});
