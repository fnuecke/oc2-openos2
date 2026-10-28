/* SPDX-License-Identifier: MIT */
/* Power off and reboot, also on request from the host, e.g. oc2's power button. */

#include "drivers/syscon.h"
#include "fs/fs.h"
#include "internal.h"

#define REQUEST_TIMEOUT_S 5

static volatile int requested;
static volatile u64 requested_at;
static interrupt_handler request_handler;

void power_request(const int request) {
    if (requested != 0) {
        return;
    }
    requested = request;
    requested_at = time_now();
    wait_wake();
    if (request_handler != NULL) {
        request_handler();
    }
}

void power_off(void) {
    fs_unmount_all();
    syscon_power_off();
    kprintf("power off failed, halting\n");
    halt();
}

void reboot(void) {
    fs_unmount_all();
    syscon_reboot();
    kprintf("reboot failed, halting\n");
    halt();
}

void halt(void) {
    csr_clear(sstatus, SSTATUS_SIE);
    csr_write(sie, 0);
    for (;;) {
        asm volatile("wfi");
    }
}

int power_requested(void) {
    return requested;
}

void power_set_interrupt_handler(const interrupt_handler handler) {
    request_handler = handler;
}

u64 power_deadline(void) {
    return requested != 0 ? requested_at + REQUEST_TIMEOUT_S * board.timebase : TIME_NEVER;
}

void power_check(void) {
    if (requested != 0 && time_now() >= power_deadline()) {
        kprintf("\n%s requested and not answered, unmounting\n", requested == POWER_OFF_REQUEST ? "power off" : "reboot");
        if (requested == POWER_OFF_REQUEST) {
            power_off();
        } else {
            reboot();
        }
    }
}
