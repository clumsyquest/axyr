#include <zephyr/kernel.h>

/* A running target. The Axyr on-device support (RTT telemetry, coredump,
 * thread analysis, context-switch trace) comes from CONFIG_AXYR — see
 * ../axyr. Here we just do work and expose a global the host can watch. */

volatile uint32_t axyr_counter = 0;

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
