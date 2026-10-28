/* SPDX-License-Identifier: MIT */
/*
 * A read-only /sys with the few Linux entries oc2's Lua libraries read.
 *
 *   /sys/class/rtc/rtc<n>/since_epoch          "1700000000", also date "2023-11-14" and time "22:13:20", UTC
 *   /sys/class/tty/ttyS<n>/iomem_base          "0x10003000"
 *   /sys/class/virtio-ports/vport0p<n>/name    the port's name, opened as /dev/vport0p<n>
 */

#include <stdio.h>
#include <string.h>
#include "drivers/rtc.h"
#include "drivers/serial.h"
#include "drivers/virtio/virtio.h"
#include "internal.h"
#include "sysfs.h"

#define CONTENT_SIZE 72

enum { NONE, IS_DIR, IS_FILE };

static struct {
    int used;
    size_t length, position;
    char content[CONTENT_SIZE];
} files[SYSFS_MAX_FILES];

static const char *next(const char *path, const char **name, size_t *len) {
    while (*path == '/') {
        path++;
    }
    if (*path == 0) {
        return NULL;
    }
    *name = path;
    while (*path && *path != '/') {
        path++;
    }
    *len = (size_t) (path - *name);
    return path;
}

static int is(const char *name, const size_t len, const char *expected) {
    return strlen(expected) == len && memcmp(name, expected, len) == 0;
}

/* ---------------------------------------------------------------------------------------- */
/* Querying */

int sysfs_port(const char *name, const size_t len) {
    if (len < 8 || memcmp(name, "vport0p", 7) != 0) {
        return -1;
    }
    int port = 0;
    for (size_t i = 7; i < len; i++) {
        if (name[i] < '0' || name[i] > '9') {
            return -1;
        }
        port = port * 10 + (name[i] - '0');
    }
    return vcon_named(port) ? port : -1;
}

static int serial_of(const char *name, const size_t len) {
    char buffer[16];
    if (len >= sizeof(buffer)) {
        return -1;
    }
    memcpy(buffer, name, len);
    buffer[len] = 0;
    return serial_find(buffer);
}

static int rtc_of(const char *name, const size_t len) {
    u64 seconds;
    return len == 4 && memcmp(name, "rtc", 3) == 0 && rtc_seconds_of(name[3] - '0', &seconds) ? name[3] - '0' : -1;
}

static int lookup(const char *path, char *content, size_t *length) {
    const char *names[4];
    size_t lens[4];
    int depth = 0;
    const char *name;
    size_t len;
    while ((path = next(path, &name, &len)) != NULL) {
        if (depth == 4) {
            return NONE;
        }
        names[depth] = name;
        lens[depth] = len;
        depth++;
    }
    if (depth == 0) {
        return IS_DIR;
    }
    if (!is(names[0], lens[0], "class")) {
        return NONE;
    }
    if (depth == 1) {
        return IS_DIR;
    }
    if (is(names[1], lens[1], "rtc")) {
        if (depth == 2) {
            return IS_DIR;
        }
        const int rtc = rtc_of(names[2], lens[2]);
        if (rtc < 0) {
            return NONE;
        }
        if (depth == 3) {
            return IS_DIR;
        }
        u64 seconds;
        rtc_seconds_of(rtc, &seconds);
        struct date d;
        civil_from_time((long) seconds, &d);
        if (is(names[3], lens[3], "since_epoch")) {
            *length = (size_t) snprintf(content, CONTENT_SIZE, "%lu\n", (unsigned long) seconds);
        } else if (is(names[3], lens[3], "date")) {
            *length = (size_t) snprintf(content, CONTENT_SIZE, "%04ld-%02ld-%02ld\n", d.year, d.month, d.day);
        } else if (is(names[3], lens[3], "time")) {
            *length = (size_t) snprintf(content, CONTENT_SIZE, "%02ld:%02ld:%02ld\n", d.hour, d.min, d.sec);
        } else {
            return NONE;
        }
        return IS_FILE;
    }
    if (is(names[1], lens[1], "tty")) {
        if (depth == 2) {
            return IS_DIR;
        }
        const int serial = serial_of(names[2], lens[2]);
        if (serial < 0) {
            return NONE;
        }
        if (depth == 3) {
            return IS_DIR;
        }
        if (!is(names[3], lens[3], "iomem_base")) {
            return NONE;
        }
        *length = (size_t) snprintf(content, CONTENT_SIZE, "0x%lX\n", serial_base(serial));
        return IS_FILE;
    }
    if (is(names[1], lens[1], "virtio-ports")) {
        if (depth == 2) {
            return IS_DIR;
        }
        const int port = sysfs_port(names[2], lens[2]);
        if (port < 0) {
            return NONE;
        }
        if (depth == 3) {
            return IS_DIR;
        }
        if (!is(names[3], lens[3], "name")) {
            return NONE;
        }
        *length = (size_t) snprintf(content, CONTENT_SIZE, "%s\n", vcon_name(port));
        return IS_FILE;
    }
    return NONE;
}

int sysfs_stat(const char *path, struct fs_stat *st) {
    char content[CONTENT_SIZE];
    size_t length = 0;
    const int kind = lookup(path, content, &length);
    if (kind == NONE) {
        return FS_NOT_FOUND;
    }
    st->size = length;
    st->mtime = 0;
    st->is_dir = kind == IS_DIR;
    return 0;
}

static void add_port(void *ctx, const int port, const char *name) {
    (void) name;
    char entry[16];
    const int len = snprintf(entry, sizeof(entry), "vport0p%d", port);
    fs_listing_add(ctx, entry, (size_t) len, 1);
}

int sysfs_list(const char *path, struct fs_listing *out) {
    char content[CONTENT_SIZE];
    size_t length;
    const int kind = lookup(path, content, &length);
    if (kind != IS_DIR) {
        return kind == IS_FILE ? FS_NOT_DIR : FS_NOT_FOUND;
    }
    const char *names[3] = {0};
    size_t lens[3] = {0};
    int depth = 0;
    const char *name;
    size_t len;
    while ((path = next(path, &name, &len)) != NULL && depth < 3) {
        names[depth] = name;
        lens[depth] = len;
        depth++;
    }
    if (depth == 0) {
        return fs_listing_add(out, "class", 5, 1);
    }
    if (depth == 1) {
        fs_listing_add(out, "rtc", 3, 1);
        fs_listing_add(out, "tty", 3, 1);
        return fs_listing_add(out, "virtio-ports", 12, 1);
    }
    if (depth == 2 && is(names[1], lens[1], "rtc")) {
        u64 seconds;
        for (int i = 0; i < 10 && rtc_seconds_of(i, &seconds); i++) {
            const char name[] = {'r', 't', 'c', (char) ('0' + i)};
            fs_listing_add(out, name, sizeof(name), 1);
        }
        return 0;
    }
    if (is(names[1], lens[1], "rtc")) {
        fs_listing_add(out, "date", 4, 0);
        fs_listing_add(out, "since_epoch", 11, 0);
        return fs_listing_add(out, "time", 4, 0);
    }
    if (depth == 2 && is(names[1], lens[1], "tty")) {
        for (int i = 0; i < serial_count(); i++) {
            fs_listing_add(out, serial_name(i), strlen(serial_name(i)), 1);
        }
        return 0;
    }
    if (depth == 2) {
        vcon_ports(add_port, out);
        return 0;
    }
    return fs_listing_add(out, is(names[1], lens[1], "tty") ? "iomem_base" : "name",
        is(names[1], lens[1], "tty") ? 10 : 4, 0);
}

/* ---------------------------------------------------------------------------------------- */
/* Opening */

sysfs_fd sysfs_open(const char *path, const int flags) {
    if (flags & (O_WRITE | O_CREATE | O_TRUNCATE | O_APPEND)) {
        return FS_READ_ONLY;
    }
    for (int fd = 0; fd < SYSFS_MAX_FILES; fd++) {
        if (!files[fd].used) {
            const int kind = lookup(path, files[fd].content, &files[fd].length);
            if (kind != IS_FILE) {
                return kind == IS_DIR ? FS_IS_DIR : FS_NOT_FOUND;
            }
            files[fd].used = 1;
            files[fd].position = 0;
            return fd;
        }
    }
    return FS_TOO_MANY;
}

/* ---------------------------------------------------------------------------------------- */
/* Open files */

long sysfs_read(const sysfs_fd fd, void *dst, const size_t len) {
    if (fd < 0 || fd >= SYSFS_MAX_FILES || !files[fd].used) {
        return FS_INVALID;
    }
    const size_t left = files[fd].length - files[fd].position;
    const size_t count = len < left ? len : left;
    memcpy(dst, files[fd].content + files[fd].position, count);
    files[fd].position += count;
    return (long) count;
}

long sysfs_seek(const sysfs_fd fd, const long offset, const int whence) {
    if (fd < 0 || fd >= SYSFS_MAX_FILES || !files[fd].used) {
        return FS_INVALID;
    }
    const long base = whence == SEEK_SET ? 0 : whence == SEEK_CUR ? (long) files[fd].position : (long) files[fd].length;
    if (base + offset < 0 || base + offset > (long) files[fd].length) {
        return FS_INVALID;
    }
    files[fd].position = (size_t) (base + offset);
    return (long) files[fd].position;
}

int sysfs_close(const sysfs_fd fd) {
    if (fd < 0 || fd >= SYSFS_MAX_FILES || !files[fd].used) {
        return FS_INVALID;
    }
    files[fd].used = 0;
    return 0;
}
