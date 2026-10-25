/* SPDX-License-Identifier: MIT */
/*
 * FILE reading is needed for lauxlib (luaL_loadfile).
 * lua-cjson logs errors with fprintf; we reroute that to the console.
 */

#include <errno.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "drivers/console.h"
#include "fs/fs.h"
#include "kernel.h"

int app_vfprintf(FILE *f, const char *format, va_list args) {
    (void) f;
    char buffer[256];
    const int len = vsnprintf(buffer, sizeof(buffer), format, args);
    console_write(buffer, len < (int) sizeof(buffer) ? (size_t) len : sizeof(buffer) - 1);
    return len;
}

int app_fprintf(FILE *f, const char *format, ...) {
    va_list args;
    va_start(args, format);
    const int len = app_vfprintf(f, format, args);
    va_end(args);
    return len;
}

/* lua-cjson only exits on errors it cannot recover from. */
void app_exit(const int code) {
    kprintf("\n*** exit(%d)\n", code);
    power_off();
}

#define FILE_BUFFER 512

struct _IO_FILE {
    int fd;
    int eof;
    int error;
    size_t fill;
    size_t pos;
    unsigned char *buffer; /* allocated by fopen, stdin and stderr have none */
};

static FILE stdin_file = {.fd = -1, .eof = 1};
static FILE stderr_file = {.fd = -1, .eof = 1};
FILE *const stdin = &stdin_file;
FILE *const stderr = &stderr_file;

/* Only debug.debug() uses these, on stdin and stderr. */
char *fgets(char *dst, const int size, FILE *f) {
    (void) dst;
    (void) size;
    (void) f;
    return NULL;
}

int fputs(const char *s, FILE *f) {
    (void) f;
    console_puts(s);
    return 0;
}

FILE *fopen(const char *path, const char *mode) {
    if (mode[0] != 'r' || strchr(mode, '+') != NULL) {
        errno = FS_INVALID;
        return NULL;
    }
    const int fd = fs_open(path, O_READ);
    if (fd < 0) {
        errno = fd;
        return NULL;
    }
    FILE *f = calloc(1, sizeof(FILE) + FILE_BUFFER);
    if (f == NULL) {
        fs_close(fd);
        errno = FS_NO_SPACE;
        return NULL;
    }
    f->fd = fd;
    f->buffer = (unsigned char *) (f + 1);
    return f;
}

FILE *freopen(const char *path, const char *mode, FILE *f) {
    if (f->fd >= 0) {
        fs_close(f->fd);
    }
    const int fd = mode[0] == 'r' && strchr(mode, '+') == NULL ? fs_open(path, O_READ) : FS_INVALID;
    if (fd < 0) {
        errno = fd;
        free(f);
        return NULL;
    }
    f->fd = fd;
    f->eof = f->error = 0;
    f->fill = f->pos = 0;
    return f;
}

int fclose(FILE *f) {
    if (f == stdin) {
        return 0;
    }
    fs_close(f->fd);
    free(f);
    return 0;
}

static int refill(FILE *f) {
    if (f->eof || f->fd < 0) {
        f->eof = 1;
        return 0;
    }
    const long count = fs_read(f->fd, f->buffer, FILE_BUFFER);
    if (count < 0) {
        f->error = 1;
        errno = (int) count;
        return 0;
    }
    if (count == 0) {
        f->eof = 1;
        return 0;
    }
    f->pos = 0;
    f->fill = (size_t) count;
    return 1;
}

int getc(FILE *f) {
    if (f->pos >= f->fill && !refill(f)) {
        return EOF;
    }
    return f->buffer[f->pos++];
}

size_t fread(void *dst, const size_t size, const size_t count, FILE *f) {
    const size_t total = size * count;
    unsigned char *out = dst;
    size_t done = 0;
    while (done < total) {
        if (f->pos >= f->fill && !refill(f)) {
            break;
        }
        size_t chunk = f->fill - f->pos;
        if (chunk > total - done) {
            chunk = total - done;
        }
        memcpy(out + done, f->buffer + f->pos, chunk);
        f->pos += chunk;
        done += chunk;
    }
    return size == 0 ? 0 : done / size;
}

int feof(FILE *f) {
    return f->eof && f->pos >= f->fill;
}

int ferror(FILE *f) {
    return f->error;
}
