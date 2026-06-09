#include <zephyr/kernel.h>

/* A running target that exposes live state for the host to read over SWD. */

volatile uint32_t axyr_counter = 0;

/* --- non-intrusive context-switch timeline ---------------------------------
 * On every thread switch-in, record (timestamp, thread-name pointer) into a RAM
 * ring buffer. This is a few instructions, no printk, no allocation — safe in
 * the scheduler hot path. The host reads the ring over SWD (background read,
 * no halt) and reconstructs "what ran when". Reuses CONFIG_TRACING_USER hooks. */
#define AXYR_TRACE_N 32
struct axyr_trace_entry {
    uint32_t timestamp;    /* k_cycle_get_32() */
    uint32_t thread_name;  /* pointer to the thread's name string */
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

int main(void)
{
    printk("live_demo running\n");
    while (1) {
        axyr_counter++;
        printk("counter=%u\n", axyr_counter);
        k_msleep(250);
    }
    return 0;
}
