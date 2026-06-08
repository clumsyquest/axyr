#include <zephyr/kernel.h>
#include <zephyr/fatal.h>

/* Our handler: Zephyr calls THIS one instead of its own, because
 * k_sys_fatal_error_handler is a weak symbol that we override here. */
void k_sys_fatal_error_handler(unsigned int reason, const struct arch_esf *esf)
{
    printk("\n>>> AXYR caught a crash! (reason=%u)\n", reason);
    printk("    pc   = 0x%08x\n", esf->basic.pc);
    printk("    lr   = 0x%08x\n", esf->basic.lr);
    printk("    xpsr = 0x%08x\n", esf->basic.xpsr);
    printk("    r0   = 0x%08x\n", esf->basic.r0);
    printk("    r1   = 0x%08x\n", esf->basic.r1);
    printk("    r2   = 0x%08x\n", esf->basic.r2);
    printk("    r3   = 0x%08x\n", esf->basic.r3);

    /* Cortex-M fault status registers are memory-mapped at fixed addresses
     * in the System Control Block — read them directly, no header needed. */
    printk("    CFSR = 0x%08x\n", *(volatile uint32_t *)0xE000ED28);
    printk("    HFSR = 0x%08x\n", *(volatile uint32_t *)0xE000ED2C);
    printk("    BFAR = 0x%08x\n", *(volatile uint32_t *)0xE000ED38);    

    printk(">>> halting\n");
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
