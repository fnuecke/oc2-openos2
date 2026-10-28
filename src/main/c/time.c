/* SPDX-License-Identifier: MIT */
#include "drivers/rtc.h"
#include "internal.h"

#define SBI_EXT_TIME 0x54494D45

static interrupt_handler watchdog_handler;
static u64 watchdog_period; /* in ticks, 0 when off */
static volatile u64 watchdog_deadline; /* 0 while waiting or off */
static volatile int woken;
static volatile u64 input_timing;

static void sbi_set_timer(const u64 value) {
    register u64 a0 asm("a0") = value;
    register u64 a6 asm("a6") = 0;
    register u64 a7 asm("a7") = SBI_EXT_TIME;
    asm volatile("ecall" : "+r"(a0) : "r"(a6), "r"(a7) : "a1", "memory");
}

void time_init(void) {
    csr_set(sie, 1ul << IRQ_S_TIMER);
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

u64 seconds_to_ticks(const double seconds) {
    const double ticks = seconds * (double) time_frequency();
    if (!(ticks > 0)) {
        return 0;
    }
    return ticks >= 0x1p62 ? 1ull << 62 : (u64) ticks;
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

/* days_from_civil and civil_from_time follow Howard Hinnant's public domain date algorithms. */
long days_from_civil(long y, const long m, const long d) {
    y -= m <= 2;
    const long era = (y >= 0 ? y : y - 399) / 400;
    const long yoe = y - era * 400;
    const long doy = (153 * (m + (m > 2 ? -3 : 9)) + 2) / 5 + d - 1;
    const long doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    return era * 146097 + doe - 719468;
}

void civil_from_time(const long t, struct date *out) {
    long days = t / 86400;
    long rest = t % 86400;
    if (rest < 0) {
        rest += 86400;
        days--;
    }
    out->hour = rest / 3600;
    out->min = rest / 60 % 60;
    out->sec = rest % 60;
    out->wday = ((days % 7) + 11) % 7 + 1; /* 1970-01-01 was a Thursday; Sunday is 1 */

    const long z = days + 719468;
    const long era = (z >= 0 ? z : z - 146096) / 146097;
    const long doe = z - era * 146097;
    const long yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    const long doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    const long mp = (5 * doy + 2) / 153;
    out->day = doy - (153 * mp + 2) / 5 + 1;
    out->month = mp < 10 ? mp + 3 : mp - 9;
    out->year = yoe + era * 400 + (out->month <= 2);
    out->yday = days - days_from_civil(out->year, 1, 1) + 1;
}
