/* SPDX-License-Identifier: MIT */
/* Serial ports: 16550s other than the console, named ttyS1 and up. */
#ifndef SERIAL_H
#define SERIAL_H

#include "kernel.h"

/* ---------------------------------------------------------------------------------------- */
/* For the kernel */

void serial_init(void);
bool serial_interrupt(u32 irq);

/* ---------------------------------------------------------------------------------------- */
/* For the application */

int serial_count(void);
int serial_find(const char *name);
const char *serial_name(int index);
u64 serial_base(int index);
int serial_set_baud(int index, u32 baud);
u32 serial_baud(int index);
bool serial_readable(int index);
bool serial_writable(int index);
/* Both return 0 when there is nothing to read or no room. */
size_t serial_read(int index, char *dst, size_t max);
size_t serial_write(int index, const char *src, size_t len);
/* Like vcon_signal_ready and vcon_take_signal, for received data. */
bool serial_signal_ready(void);
int serial_take_signal(void);

#endif
