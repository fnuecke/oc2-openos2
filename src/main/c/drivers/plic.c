/* SPDX-License-Identifier: MIT */
/* The platform-level interrupt controller (PLIC): enables sources and dispatches their interrupts. */

#include "drivers/virtio/virtio.h"
#include "console.h"
#include "internal.h"
#include "plic.h"
#include "serial.h"
#include "syscon.h"

#define PLIC_PRIORITY 0x0
#define PLIC_ENABLE 0x2000
#define PLIC_ENABLE_STRIDE 0x80
#define PLIC_THRESHOLD 0x200000
#define PLIC_CLAIM 0x200004
#define PLIC_CONTEXT_STRIDE 0x1000

static u64 context_base(void) {
    return board.plic + (u64) board.plic_context * PLIC_CONTEXT_STRIDE;
}

void plic_init(void) {
    if (board.plic == 0 || board.plic_context < 0) {
        kprintf("openos2: no PLIC with an S-mode context in the device tree, halting\n");
        halt();
    }
    mmio_write32(context_base() + PLIC_THRESHOLD, 0);
    csr_set(sie, 1ul << IRQ_S_EXTERNAL);
}

void plic_enable(const u32 irq) {
    if (irq == 0) {
        return;
    }
    mmio_write32(board.plic + PLIC_PRIORITY + sizeof(u32) * irq, 1);
    const u64 context_enable = board.plic + PLIC_ENABLE + (u64) board.plic_context * PLIC_ENABLE_STRIDE;
    const u64 enable = context_enable + sizeof(u32) * (irq / 32);
    mmio_write32(enable, mmio_read32(enable) | (1u << (irq % 32)));
}

void plic_dispatch(void) {
    const u32 claimed = mmio_read32(context_base() + PLIC_CLAIM);
    if (claimed == 0) {
        return;
    }
    if (!console_interrupt(claimed) && !serial_interrupt(claimed) && !syscon_interrupt(claimed)) {
        virtio_interrupt(claimed);
    }
    mmio_write32(context_base() + PLIC_CLAIM, claimed);
}
