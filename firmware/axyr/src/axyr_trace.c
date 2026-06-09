/*
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
