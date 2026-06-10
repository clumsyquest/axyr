// ============================================================
// Axyr Cockpit — real firmware source, verbatim from the repo.
// Used by the Agent workspace file tree + code viewer.
// ============================================================

const F_CRASH_MAIN = `#include <zephyr/kernel.h>
#include <zephyr/fatal.h>

void k_sys_fatal_error_handler(unsigned int reason, const struct arch_esf *esf)
{
    /* Emit one structured, parsable line: the crash "packet" the host reads.
     * The AXYR_CRASH prefix lets the host find it amid other serial output.
     * This is the fast path; the full call stack comes from the coredump
     * block (#CD:...) that Zephyr also emits when CONFIG_DEBUG_COREDUMP=y. */
    printk("AXYR_CRASH v=1 reason=%u pc=0x%08x lr=0x%08x xpsr=0x%08x "
           "r0=0x%08x r1=0x%08x r2=0x%08x r3=0x%08x r12=0x%08x\\n",
           reason,
           esf->basic.pc, esf->basic.lr, esf->basic.xpsr,
           esf->basic.r0, esf->basic.r1, esf->basic.r2, esf->basic.r3,
           esf->basic.r12);

    k_fatal_halt(reason);
}

/* Nested call chain so the captured backtrace shows a real path to the fault,
 * not just the crashing line. noinline keeps the optimizer from collapsing
 * these frames into main(). */

/* Lowest level: pretends to read a hardware register, dereferences a bad
 * pointer, and faults. 0xBADCAFE0 is unmapped on the STM32F401, so reading
 * it raises a precise bus fault. */
static uint32_t __attribute__((noinline)) i2c_read_reg(uint8_t reg)
{
    volatile uint32_t *bad_ptr = (volatile uint32_t *)(0xBADCAFE0 + reg);
    return *bad_ptr;
}

/* Mid level: a sensor driver that talks to the bus. */
static uint32_t __attribute__((noinline)) read_sensor(void)
{
    return i2c_read_reg(0x00);
}

int main(void)
{
    printk("About to crash...\\n");

    uint32_t value = read_sensor();

    printk("Never reached: %u\\n", value);
    return 0;
}
`;

const F_LIVE_MAIN = `#include <zephyr/kernel.h>

/* A running target. The Axyr on-device support (RTT telemetry, coredump,
 * thread analysis, context-switch trace) comes from CONFIG_AXYR — see
 * ../axyr. Here we just do work and expose a global the host can watch. */

volatile uint32_t axyr_counter = 0;

int main(void)
{
	printk("live_demo running\\n");
	while (1) {
		axyr_counter++;
		printk("counter=%u\\n", axyr_counter);
		k_msleep(250);
	}
	return 0;
}
`;

const F_PRJ = `# One switch wires in all Axyr on-device support (RTT telemetry, in-memory
# coredump, thread analysis, trace ring). See ../axyr.
CONFIG_AXYR=y
`;

const F_CMAKE = `cmake_minimum_required(VERSION 3.20.0)
find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(crash_demo)

target_sources(app PRIVATE src/main.c)
`;

const F_TRACE = `/*
 * Axyr on-device trace: a RAM ring buffer of thread switch-ins, written from a
 * CONFIG_TRACING_USER hook (a few instructions, no printk — safe in the
 * scheduler hot path). The host reads the ring over SWD to reconstruct "what
 * ran when". Compiled into any firmware that enables CONFIG_AXYR.
 */
#include <zephyr/kernel.h>

#define AXYR_TRACE_N 32
struct axyr_trace_entry {
	uint32_t timestamp;   /* k_cycle_get_32() */
	uint32_t thread_name; /* pointer to the thread's name string */
};
volatile struct axyr_trace_entry axyr_trace[AXYR_TRACE_N];
volatile uint32_t axyr_trace_head; /* total writes; index = head % N */

void sys_trace_thread_switched_in_user(void)
{
	struct k_thread *t = k_current_get();
	uint32_t i = axyr_trace_head % AXYR_TRACE_N;

	axyr_trace[i].timestamp = k_cycle_get_32();
	axyr_trace[i].thread_name = (uint32_t)(uintptr_t)k_thread_name_get(t);
	axyr_trace_head++;
}
`;

// File tree: real paths from the repo. lang drives syntax tint.
const FILE_TREE = [
  { type: 'dir', name: 'firmware', open: true, children: [
    { type: 'dir', name: 'crash_demo', open: true, children: [
      { type: 'dir', name: 'src', open: true, children: [
        { type: 'file', name: 'main.c', path: 'firmware/crash_demo/src/main.c', lang: 'c', body: F_CRASH_MAIN, crash: true },
      ]},
      { type: 'file', name: 'prj.conf', path: 'firmware/crash_demo/prj.conf', lang: 'conf', body: F_PRJ },
      { type: 'file', name: 'CMakeLists.txt', path: 'firmware/crash_demo/CMakeLists.txt', lang: 'cmake', body: F_CMAKE },
    ]},
    { type: 'dir', name: 'live_demo', children: [
      { type: 'dir', name: 'src', children: [
        { type: 'file', name: 'main.c', path: 'firmware/live_demo/src/main.c', lang: 'c', body: F_LIVE_MAIN },
      ]},
    ]},
    { type: 'dir', name: 'axyr', children: [
      { type: 'dir', name: 'src', children: [
        { type: 'file', name: 'axyr_trace.c', path: 'firmware/axyr/src/axyr_trace.c', lang: 'c', body: F_TRACE },
      ]},
    ]},
  ]},
];

const OPEN_FILE_DEFAULT = 'firmware/crash_demo/src/main.c';

// flatten helper -> map path -> file node
function flattenFiles(tree, acc) {
  acc = acc || {};
  for (const n of tree) {
    if (n.type === 'file') acc[n.path] = n;
    else if (n.children) flattenFiles(n.children, acc);
  }
  return acc;
}

Object.assign(window, {
  AX_FILE_TREE: FILE_TREE,
  AX_FILES: flattenFiles(FILE_TREE),
  AX_OPEN_DEFAULT: OPEN_FILE_DEFAULT,
});
