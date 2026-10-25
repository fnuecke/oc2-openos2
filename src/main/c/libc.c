/* SPDX-License-Identifier: MIT */
/*
 * The bits of the C library we have to do ourself, instead of relying on musl. Stuff that would
 * need to make a system call or read the thread pointer. For the rest we do just use musl's
 * libc.a (string, ctype, math, snprintf, strtod, setjmp).
 */

#include <errno.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <wchar.h>
#include "drivers/console.h"
#include "drivers/rtc.h"
#include "fs/fs.h"
#include "internal.h"

static int errno_value;

int *__errno_location(void) {
    return &errno_value;
}

int *___errno_location(void) __attribute__((alias("__errno_location")));

int __lockfile(FILE *f) {
    (void) f;
    return 0;
}

void __unlockfile(FILE *f) {
    (void) f;
}

void __assert_fail(const char *expression, const char *file, const int line, const char *function) {
    kprintf("\n*** assertion failed: %s (%s:%d, %s)\n", expression, file, line, function);
    halt();
}

void abort(void) {
    kprintf("\n*** abort\n");
    halt();
}

time_t time(time_t *out) {
    const time_t now = (time_t) rtc_seconds();
    if (out != NULL) {
        *out = now;
    }
    return now;
}

clock_t clock(void) {
    return (clock_t) (time_now() / (board.timebase / CLOCKS_PER_SEC));
}

char *getenv(const char *name) {
    (void) name;
    return NULL;
}

char *strerror(const int code) {
    static char buffer[32];
    if (code < 0) {
        return (char *) fs_error(code);
    }
    snprintf(buffer, sizeof(buffer), "Unknown error %d", code);
    return buffer;
}

int strcoll(const char *a, const char *b) {
    return strcmp(a, b);
}

int printf(const char *format, ...) {
    char buffer[256];
    va_list args;
    va_start(args, format);
    const int len = vsnprintf(buffer, sizeof(buffer), format, args);
    va_end(args);
    console_write(buffer, len < (int) sizeof(buffer) ? (size_t) len : sizeof(buffer) - 1);
    return len;
}

size_t wcrtomb(char *dst, const wchar_t c, mbstate_t *state) {
    (void) state;
    if (dst == NULL) {
        return 1;
    }
    if ((unsigned) c >= 0x80) {
        errno = EILSEQ;
        return (size_t) -1;
    }
    *dst = (char) c;
    return 1;
}
