/* SPDX-License-Identifier: MIT */
/* The console: a 16550 UART, the first one or /chosen's stdout-path. kprintf is in kernel.h. */
#ifndef CONSOLE_H
#define CONSOLE_H

#include "kernel.h"

/* ---------------------------------------------------------------------------------------- */
/* For the kernel */

void console_init(void);
bool console_interrupt(u32 irq);

/* ---------------------------------------------------------------------------------------- */
/* For the application */

void console_write(const char *s, size_t len);
void console_puts(const char *s);
bool console_has_input(void);
size_t console_read(char *dst, size_t max, u64 deadline);

/*
 * Ctrl+C: drops the input typed before it, ends the current wait_until() and calls `handler`.
 * The application must call console_clear_interrupt() once it handled it.
 */
void console_set_interrupt_handler(interrupt_handler handler);
void console_clear_interrupt(void);
u32 console_interrupts(void); /* Ctrl+C presses so far */

#endif
