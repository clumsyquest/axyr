#include <zephyr/kernel.h>
#include <zephyr/fatal.h>

void k_sys_fatal_error_handler(unsigned int reason, const struct arch_esf *esf)
{
    /* Emit one structured, parsable line: the crash "packet" the host reads.
     * The AXYR_CRASH prefix lets the host find it amid other serial output.
     * This is the fast path; the full call stack comes from the coredump
     * block (#CD:...) that Zephyr also emits when CONFIG_DEBUG_COREDUMP=y. */
    printk("AXYR_CRASH v=1 reason=%u pc=0x%08x lr=0x%08x xpsr=0x%08x "
           "r0=0x%08x r1=0x%08x r2=0x%08x r3=0x%08x r12=0x%08x\n",
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
    printk("About to crash...\n");

    uint32_t value = read_sensor();

    printk("Never reached: %u\n", value);
    return 0;
}
