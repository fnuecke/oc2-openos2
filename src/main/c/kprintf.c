/* SPDX-License-Identifier: MIT */

#include <stdarg.h>
#include <stdio.h>
#include "kernel.h"

/* Writes ignoring stdout's buffer, also from a trap or a panic. */
void kprintf(const char *format, ...) {
    char buffer[256];
    va_list args;
    va_start(args, format);
    const int len = vsnprintf(buffer, sizeof(buffer), format, args);
    va_end(args);
    console_write(buffer, len < (int) sizeof(buffer) ? (size_t) len : sizeof(buffer) - 1);
}
