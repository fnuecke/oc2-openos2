/* SPDX-License-Identifier: MIT */

#include "internal.h"
#include "plic.h"
#include "syscon.h"

#define SYSCON_CONTROL 0x08
#define SYSCON_STATUS 0x0C
#define CONTROL_ENABLE (1u << 1)
#define STATUS_POWEROFF (1u << 0)
#define STATUS_RESET (1u << 1)

void syscon_init(void) {
    if (board.syscon == 0) {
        return; /* the host stops the machine instead */
    }
    mmio_write32(board.syscon + SYSCON_CONTROL, CONTROL_ENABLE);
    plic_enable(board.syscon_irq);
}

bool syscon_interrupt(const u32 irq) {
    if (board.syscon == 0 || irq != board.syscon_irq) {
        return 0;
    }
    const u32 status = mmio_read32(board.syscon + SYSCON_STATUS);
    mmio_write32(board.syscon + SYSCON_STATUS, status); /* clears it, and the interrupt */
    if (status & STATUS_POWEROFF) {
        power_request(POWER_OFF_REQUEST);
    } else if (status & STATUS_RESET) {
        power_request(POWER_REBOOT_REQUEST);
    }
    return 1;
}

void syscon_power_off(void) {
    if (board.poweroff_addr != 0) {
        mmio_write32(board.poweroff_addr, board.poweroff_value);
    }
}

void syscon_reboot(void) {
    if (board.reboot_addr != 0) {
        mmio_write32(board.reboot_addr, board.reboot_value);
    }
}
