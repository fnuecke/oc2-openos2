/* SPDX-License-Identifier: MIT */
#include "drivers/rtc.h"
#include "internal.h"

#define SIE_STIE (1ul << 5)
#define SBI_EXT_TIME 0x54494D45

static interrupt_handler watchdog_handler;
static u64 watchdog_period; /* in ticks, 0 when off */
static volatile u64 watchdog_deadline; /* 0 while waiting or off */
static volatile int woken; /* ends the current wait_until() early */
static volatile u64 input_timing; /* see entropy_add */

static void sbi_set_timer(const u64 value) {
    register u64 a0 asm("a0") = value;
    register u64 a6 asm("a6") = 0;
    register u64 a7 asm("a7") = SBI_EXT_TIME;
    asm volatile("ecall" : "+r"(a0) : "r"(a6), "r"(a7) : "a1", "memory");
}

void time_init(void) {
    csr_set(sie, SIE_STIE);
    sbi_set_timer(TIME_NEVER);
}

void time_interrupt(void) {
    sbi_set_timer(TIME_NEVER); /* clears the pending bit; wait_until() rechecks the deadline */
    if (watchdog_deadline != 0 && time_now() >= watchdog_deadline) {
        watchdog_handler();
        watchdog_deadline = time_now() + watchdog_period; /* again, if the error is caught */
    }
    if (watchdog_deadline != 0) {
        sbi_set_timer(watchdog_deadline);
    }
}

void wait_wake(void) {
    woken = 1;
}

void wait_clear_wake(void) {
    woken = 0;
}

void entropy_add(const u64 value) {
    input_timing = (input_timing ^ value) * 0x100000001B3ull;
}

u64 entropy(void) {
    u64 value = time_now() ^ input_timing;
    for (int i = 0; i < board.rtc_count; i++) {
        value = (value ^ rtc_read(i)) * 0x100000001B3ull;
    }
    return value;
}

u64 mix64(u64 x) {
    x += 0x9E3779B97F4A7C15ull;
    x = (x ^ (x >> 30)) * 0xBF58476D1CE4E5B9ull;
    x = (x ^ (x >> 27)) * 0x94D049BB133111EBull;
    return x ^ (x >> 31);
}

u64 time_frequency(void) {
    return board.timebase;
}

static void arm_watchdog(void) {
    watchdog_deadline = watchdog_period != 0 ? time_now() + watchdog_period : 0;
    sbi_set_timer(watchdog_period != 0 ? watchdog_deadline : TIME_NEVER);
}

bool wait_until(bool (*ready)(void *ctx), void *ctx, const u64 deadline) {
    int result = 0;
    watchdog_deadline = 0;
    for (;;) {
        power_check();
        csr_clear(sstatus, SSTATUS_SIE);
        const u64 now = time_now();
        if (ready != NULL && ready(ctx)) {
            result = 1;
            break;
        }
        if (woken) {
            woken = 0;
            break;
        }
        if (now >= deadline) {
            break;
        }
        sbi_set_timer(deadline < power_deadline() ? deadline : power_deadline());
        asm volatile("wfi");
        csr_set(sstatus, SSTATUS_SIE);
    }
    csr_set(sstatus, SSTATUS_SIE);
    arm_watchdog();
    return result;
}

void sleep_until(const u64 deadline) {
    wait_until(NULL, NULL, deadline);
}

void watchdog_set(const u64 period, const interrupt_handler handler) {
    watchdog_period = handler != NULL ? period : 0;
    watchdog_handler = handler;
    arm_watchdog();
}

void watchdog_restart(void) {
    arm_watchdog();
}
