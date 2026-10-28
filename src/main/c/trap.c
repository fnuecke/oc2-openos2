/* SPDX-License-Identifier: MIT */
/* See start.S for how this is called. */

#include "drivers/plic.h"
#include "internal.h"

#define SCAUSE_INTERRUPT (1ul << 63)

extern u8 _stack_guard[], _stack_bottom[];

void handle_interrupt(const u64 cause) {
    switch (cause & ~SCAUSE_INTERRUPT) {
        case IRQ_S_TIMER:
            time_interrupt();
            break;
        case IRQ_S_EXTERNAL:
            plic_dispatch();
            break;
    }
}

void trap_report(const u64 cause, const u64 epc, const u64 tval, const u64 sp, const u64 ra) {
    if (tval >= (u64) _stack_guard && tval < (u64) _stack_bottom) {
        kprintf("\n*** kernel stack overflow");
    }
    kprintf("\n*** kernel trap: scause=%#lx sepc=%#lx stval=%#lx sp=%#lx ra=%#lx\n", cause, epc, tval, sp, ra);
    kprintf("*** system halted\n");
    halt();
}
