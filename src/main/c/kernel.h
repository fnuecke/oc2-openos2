/* SPDX-License-Identifier: MIT */
/*
 * The kernel's interface to the application that runs on top of it. After boot the kernel calls
 * app_main(); when it returns, the kernel powers off. Everything the application needs is declared
 * here, the kernel's internals are in internal.h.
 *
 * There are no threads and no preemption: the application runs on the one stack, and the kernel
 * runs only when called or from interrupt handlers. Waiting is done with wait_until(), which sleeps
 * in wfi until a condition holds. The application on top of the kernel can set three interrupt
 * handlers: console_set_interrupt_handler() for Ctrl+C, watchdog_set() for code that runs too long,
 * and power_set_interrupt_handler() for the host's power button.
 *
 * The callbacks run in the trap handler: with interrupts masked, on the stack of the code they
 * interrupted, which may be anywhere in the application or the kernel. They may only set flags or
 * do what is safe at any instruction (e.g. Lua's lua_sethook is), and must not call the functions
 * here or allocate memory. The application acts on the flag once the interrupted code continues.
 */
#ifndef KERNEL_H
#define KERNEL_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

typedef uint8_t u8;
typedef uint16_t u16;
typedef uint32_t u32;
typedef uint64_t u64;

/* To be provided by the application running on top of the kernel. */
void app_main(void);

typedef void (*interrupt_handler)(void);

/* ---------------------------------------------------------------------------------------- */
/* Time, in ticks of the time CSR */

static inline u64 time_now(void) {
    u64 value;
    asm volatile("rdtime %0" : "=r"(value));
    return value;
}
#define TIME_NEVER (~0ull)

/* Ticks per second. */
u64 time_frequency(void);

/* Saturates, so that adding the result to the current time cannot wrap. NaN gives 0. */
u64 seconds_to_ticks(double seconds);

/*
 * Sleeps until ready(ctx) returns true, the deadline passes, or Ctrl+C is pressed.
 * ready is called with interrupts masked; may be NULL.
 * Returns whether ready returned true.
 */
bool wait_until(bool (*ready)(void *ctx), void *ctx, u64 deadline);
void sleep_until(u64 deadline);

/*
 * Calls `handler` from the timer interrupt when the application runs for `period` ticks
 * without a wait_until() in between. A period of 0 or no handler turns it off.
 */
void watchdog_set(u64 period, interrupt_handler handler);
void watchdog_restart(void);

/* UTC dates and seconds since 1970. wday is 1 for Sunday, yday 1 for January 1. */
struct date {
    long year, month, day, hour, min, sec, wday, yday;
};

long days_from_civil(long y, long m, long d);
void civil_from_time(long t, struct date *out);

/* ---------------------------------------------------------------------------------------- */
/* Console */

void kprintf(const char *format, ...);

/* ---------------------------------------------------------------------------------------- */
/* Memory */

size_t heap_total(void);
size_t heap_used(void);
/* malloc, realloc and free come from the C library, on the kernel's heap. */
void stack_usage(size_t *peak, size_t *size);

/* ---------------------------------------------------------------------------------------- */
/* Power */

void power_off(void) __attribute__((noreturn));
void reboot(void) __attribute__((noreturn));
void halt(void) __attribute__((noreturn));

#define POWER_OFF_REQUEST 1
#define POWER_REBOOT_REQUEST 2

/*
 * The host asks to power off or reboot through Sedna's system controller. The kernel ends the
 * current wait_until() and calls `handler`. The application must then end what it is doing
 * and call power_off() or reboot(), which then unmount the file systems. The application has
 * 5 seconds to do so, otherwise the kernel does it itself, at the next wait_until() or file
 * system call. Returns POWER_OFF_REQUEST, POWER_REBOOT_REQUEST or 0.
 */
int power_requested(void);
void power_set_interrupt_handler(interrupt_handler handler);
u64 power_deadline(void); /* when the kernel acts on a request, TIME_NEVER without one */
/*
 * Powers off or reboots if a request is past the deadline. Must only be called when no file
 * system call is in flight. wait_until() and the file system calls call it, and an application
 * may too, e.g. between two instructions of an interpreter.
 */
void power_check(void);

#endif
