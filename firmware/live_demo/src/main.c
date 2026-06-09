#include <zephyr/kernel.h>

/* A moving target so the host can prove it reads LIVE state over SWD while the
 * core runs. Nothing is special about this firmware: the engine resolves
 * `axyr_counter` from the ELF symbol table and reads its address live — exactly
 * what it does for any real application's globals. This is only a demo to make
 * the capability visible. */

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
