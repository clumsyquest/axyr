#include <zephyr/kernel.h>

int main(void)
{
    printk("About to crash...\n");

    /* 0xBADCAFE0 n'existe pas sur le STM32F401 : lire ici lève une faute */
    volatile uint32_t *bad_ptr = (volatile uint32_t *)0xBADCAFE0;
    uint32_t value = *bad_ptr;            /* <-- LA ligne fautive */

    printk("Jamais atteint : %u\n", value);
    return 0;
}
