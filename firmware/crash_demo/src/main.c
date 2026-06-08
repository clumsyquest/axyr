#include <zephyr/kernel.h>
#include <zephyr/fatal.h>

void k_sys_fatal_error_handler(unsigned int reason, const struct arch_esf *esf)
{
    /* Emit one structured, parsable line: the crash "packet" the host reads.
     * The AXYR_CRASH prefix lets the host find it amid other serial output. */
    printk("AXYR_CRASH v=1 reason=%u pc=0x%08x lr=0x%08x xpsr=0x%08x "
           "r0=0x%08x r1=0x%08x r2=0x%08x r3=0x%08x r12=0x%08x\n",
           reason,
           esf->basic.pc, esf->basic.lr, esf->basic.xpsr,
           esf->basic.r0, esf->basic.r1, esf->basic.r2, esf->basic.r3,
           esf->basic.r12);

    k_fatal_halt(reason);
}

int main(void)
{
    printk("About to crash...\n");

    /* 0xBADCAFE0 is not mapped on the STM32F401: reading it raises a fault */
    volatile uint32_t *bad_ptr = (volatile uint32_t *)0xBADCAFE0;
    uint32_t value = *bad_ptr;

    printk("Never reached: %u\n", value);
    return 0;
}
