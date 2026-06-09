#include <zephyr/kernel.h>

/* Pushes telemetry over RTT every 250 ms, sleeping (WFI) in between — to test
 * that the host reads it non-intrusively even while the core sleeps. */
int main(void)
{
    uint32_t counter = 0;
    while (1) {
        printk("counter=%u\n", counter++);
        k_msleep(250);
    }
    return 0;
}
