/* SPDX-License-Identifier: MIT */
/*
 * Dispatches the fs_* API to ext2 (/, /mnt/<label> and /mnt/<device index>), 9p (/mnt/<mount tag>),
 * sysfs (/sys) or devfs (/dev). Backend file descriptors are offset into contiguous, ascending ranges.
 */

#include <stdlib.h>
#include <string.h>
#include "devfs.h"
#include "ext2.h"
#include "internal.h"
#include "p9.h"
#include "sysfs.h"
#include "vfs.h"

#define SYS_FD_BASE EXT2_MAX_FILES
#define P9_FD_BASE (SYS_FD_BASE + SYSFS_MAX_FILES)
#define RAW_FD_BASE (P9_FD_BASE + P9_MAX_FILES)
#define DEV_FD_BASE (RAW_FD_BASE + EXT2_RAW_HANDLES)

#define NORMALIZE(path, buffer) \
    char buffer[FS_PATH_LENGTH]; \
    do { \
        power_check(); \
        const int normalized = fs_normalize(path, buffer); \
        if (normalized != 0) { \
            return normalized; \
        } \
        path = buffer; \
    } while (0)

#define MOUNT_POINT(path, buffer) \
    char buffer[EXT2_PATH_LENGTH]; \
    do { \
        const char *dev = dev_path(path); \
        u32 mount_serial; \
        if (dev != NULL) { \
            const int found = devfs_mount_point(dev, buffer, &mount_serial); \
            if (found != 0) { \
                return found; \
            } \
            path = buffer; \
        } \
    } while (0)

#define MAX_MEDIUM_EVENTS 16

static struct {
    char path[FS_MOUNT_PATH_LENGTH];
    const char *what;
} medium_events[MAX_MEDIUM_EVENTS];
static u32 medium_head, medium_tail;

/* ---------------------------------------------------------------------------------------- */
/* For the kernel */

void fs_mount_all(void) {
    p9_mount_all();
    ext2_mount_all();
}

void fs_medium_event(const char *path, const char *what) {
    if (medium_head - medium_tail == MAX_MEDIUM_EVENTS) {
        return;
    }
    char *slot = medium_events[medium_head % MAX_MEDIUM_EVENTS].path;
    strncpy(slot, path, sizeof(medium_events[0].path) - 1);
    slot[sizeof(medium_events[0].path) - 1] = 0;
    medium_events[medium_head % MAX_MEDIUM_EVENTS].what = what;
    medium_head++;
}

int fs_listing_add(struct fs_listing *out, const char *name, const size_t len, const bool is_dir) {
    if (len > 255) {
        return FS_INVALID;
    }
    if (out->length + 2 + len > out->capacity) {
        const size_t capacity = out->capacity * 2 + 256;
        u8 *data = realloc(out->data, capacity);
        if (data == NULL) {
            return FS_NO_SPACE;
        }
        out->data = data;
        out->capacity = capacity;
    }
    out->data[out->length] = (u8) (is_dir != 0);
    out->data[out->length + 1] = (u8) len;
    memcpy(out->data + out->length + 2, name, len);
    out->length += 2 + len;
    return 0;
}

int fs_listing_add_unique(struct fs_listing *out, const char *name, const size_t len, const bool is_dir) {
    for (size_t at = 0; at < out->length; at += 2 + out->data[at + 1]) {
        if (out->data[at + 1] == len && memcmp(out->data + at + 2, name, len) == 0) {
            return 0;
        }
    }
    return fs_listing_add(out, name, len, is_dir);
}

/* ---------------------------------------------------------------------------------------- */
/* Mounts and media */

const char *fs_error(const int code) {
    switch (code) {
        case FS_EXISTS: return "File exists";
        case FS_NOT_FOUND: return "No such file or directory";
        case FS_NOT_DIR: return "Not a directory";
        case FS_IS_DIR: return "Is a directory";
        case FS_NOT_EMPTY: return "Directory not empty";
        case FS_NO_SPACE: return "No space left on device";
        case FS_READ_ONLY: return "Read-only file system";
        case FS_INVALID: return "Invalid argument";
        case FS_IO: return "Input/output error";
        case FS_TOO_MANY: return "Too many open files";
        case FS_CHANGED: return "Medium changed";
        case FS_BUSY: return "Device or resource busy";
        case FS_UNFORMATTED: return "Medium not formatted";
        case FS_UNSUPPORTED: return "Unsupported file system";
        case FS_IS_ROOT: return "Is the boot medium";
        case FS_NOT_BLANK: return "Medium holds data";
        case FS_NO_MEDIUM: return "No medium found";
        default: return "Unknown error";
    }
}

int fs_normalize(const char *path, char *out) {
    size_t n = 0;
    out[n++] = '/';
    const char *p = path;
    while (*p) {
        while (*p == '/') {
            p++;
        }
        const char *name = p;
        while (*p && *p != '/') {
            p++;
        }
        const size_t len = (size_t) (p - name);
        if (len == 0 || (len == 1 && name[0] == '.')) {
            continue;
        }
        if (len == 2 && name[0] == '.' && name[1] == '.') {
            while (n > 1 && out[n - 1] != '/') {
                n--;
            }
            if (n > 1) {
                n--;
            }
            continue;
        }
        if (n + len + 2 >= FS_PATH_LENGTH) {
            return FS_INVALID;
        }
        if (n > 1) {
            out[n++] = '/';
        }
        memcpy(out + n, name, len);
        n += len;
    }
    const size_t length = strlen(path);
    if (n > 1 && length > 0 && path[length - 1] == '/') {
        out[n++] = '/';
    }
    out[n] = 0;
    return 0;
}

void fs_mounts(const fs_mounts_fn fn, void *ctx) {
    ext2_mounts(fn, ctx);
    p9_mounts(fn, ctx);
}

void fs_unmount_all(void) {
    ext2_unmount_all();
}

static const char *sys_path(const char *path) {
    return strncmp(path, "/sys", 4) == 0 && (path[4] == 0 || path[4] == '/') ? path + 4 : NULL;
}

static const char *dev_path(const char *path) {
    return strncmp(path, "/dev", 4) == 0 && (path[4] == 0 || path[4] == '/') ? path + 4 : NULL;
}

static bool is_virtual(const char *path) {
    const char *rest;
    return sys_path(path) != NULL || dev_path(path) != NULL || p9_resolve(path, &rest) != NULL;
}

int fs_unmount(const char *path, const u32 serial) {
    NORMALIZE(path, normalized_path);
    MOUNT_POINT(path, mount_point);
    return is_virtual(path) ? FS_INVALID : ext2_unmount(path, serial);
}

int fs_mount(const char *path) {
    NORMALIZE(path, normalized_path);
    MOUNT_POINT(path, mount_point);
    return is_virtual(path) ? FS_INVALID : ext2_mount(path);
}

void fs_poll_media(void) {
    ext2_poll_media();
}

int fs_take_medium_event(char *path, const size_t size, const char **what) {
    if (medium_head == medium_tail || size == 0) {
        return 0;
    }
    strncpy(path, medium_events[medium_tail % MAX_MEDIUM_EVENTS].path, size - 1);
    path[size - 1] = 0;
    *what = medium_events[medium_tail % MAX_MEDIUM_EVENTS].what;
    medium_tail++;
    return 1;
}

bool fs_medium_event_pending(void) {
    return medium_head != medium_tail;
}

/* ---------------------------------------------------------------------------------------- */
/* Querying */

int fs_info(const char *path, u64 *total, u64 *free) {
    NORMALIZE(path, normalized_path);
    MOUNT_POINT(path, mount_point);
    if (sys_path(path) != NULL) {
        return FS_INVALID;
    }
    const char *rest;
    struct p9 *m = p9_resolve(path, &rest);
    return m != NULL ? p9_info(m, total, free) : ext2_info(path, total, free);
}

int fs_stat(const char *path, struct fs_stat *st) {
    NORMALIZE(path, normalized_path);
    *st = (struct fs_stat) {0};
    if (dev_path(path) != NULL) {
        return devfs_stat(dev_path(path), st);
    }
    if (sys_path(path) != NULL) {
        return sysfs_stat(sys_path(path), st);
    }
    const char *rest;
    struct p9 *m = p9_resolve(path, &rest);
    return m != NULL ? p9_stat(m, rest, st) : ext2_stat(path, st);
}

static bool is_mnt(const char *path) {
    while (*path == '/') {
        path++;
    }
    return strncmp(path, "mnt", 3) == 0 && (path[3] == 0 || strcmp(path + 3, "/") == 0);
}

int fs_list(const char *path, struct fs_listing *out) {
    NORMALIZE(path, normalized_path);
    if (dev_path(path) != NULL) {
        return devfs_list(dev_path(path), out);
    }
    if (sys_path(path) != NULL) {
        return sysfs_list(sys_path(path), out);
    }
    const char *rest;
    struct p9 *m = p9_resolve(path, &rest);
    if (m != NULL) {
        return p9_list(m, rest, out);
    }
    const int result = ext2_list(path, out);
    if (result == 0 && strcmp(path, "/") == 0) {
        fs_listing_add_unique(out, "dev", 3, 1);
        fs_listing_add_unique(out, "sys", 3, 1);
    }
    if (is_mnt(path)) {
        ext2_mount_names(out);
        p9_names(out);
        return 0;
    }
    return result;
}

int fs_label(const char *path, char *label) {
    NORMALIZE(path, normalized_path);
    MOUNT_POINT(path, mount_point);
    if (sys_path(path) != NULL) {
        return FS_INVALID;
    }
    const char *rest;
    struct p9 *m = p9_resolve(path, &rest);
    return m != NULL ? p9_label(m, rest, label) : ext2_label(path, label);
}

/* ---------------------------------------------------------------------------------------- */
/* Opening */

fs_fd fs_open(const char *path, const int flags) {
    NORMALIZE(path, normalized_path);
    if (dev_path(path) != NULL) {
        const int fd = devfs_open(dev_path(path), flags);
        return fd < 0 ? fd : DEV_FD_BASE + fd;
    }
    const char *sys = sys_path(path);
    if (sys != NULL) {
        const int fd = sysfs_open(sys, flags);
        return fd < 0 ? fd : SYS_FD_BASE + fd;
    }
    const char *rest;
    struct p9 *m = p9_resolve(path, &rest);
    if (m == NULL) {
        return ext2_open(path, flags);
    }
    const int fd = p9_open(m, rest, flags);
    return fd < 0 ? fd : P9_FD_BASE + fd;
}

fs_fd fs_open_raw(const char *path, const int flags, const bool force, const u32 serial) {
    NORMALIZE(path, normalized_path);
    MOUNT_POINT(path, mount_point);
    if (is_virtual(path)) {
        return FS_INVALID;
    }
    const int fd = ext2_open_raw(path, flags, force, serial);
    return fd < 0 ? fd : RAW_FD_BASE + fd;
}

/* ---------------------------------------------------------------------------------------- */
/* Modifying */

int fs_mkdir(const char *path) {
    NORMALIZE(path, normalized_path);
    if (sys_path(path) != NULL || dev_path(path) != NULL) {
        return FS_READ_ONLY;
    }
    const char *rest;
    struct p9 *m = p9_resolve(path, &rest);
    return m != NULL ? p9_mkdir(m, rest) : ext2_mkdir(path);
}

int fs_rename(const char *from, const char *to) {
    NORMALIZE(from, normalized_from);
    NORMALIZE(to, normalized_to);
    if (sys_path(from) != NULL || sys_path(to) != NULL || dev_path(from) != NULL || dev_path(to) != NULL) {
        return FS_READ_ONLY;
    }
    const char *from_rest, *to_rest;
    struct p9 *from_mount = p9_resolve(from, &from_rest);
    struct p9 *to_mount = p9_resolve(to, &to_rest);
    if (from_mount != to_mount) {
        return FS_INVALID;
    }
    return from_mount != NULL ? p9_rename(from_mount, from_rest, to_rest) : ext2_rename(from, to);
}

int fs_remove(const char *path) {
    NORMALIZE(path, normalized_path);
    if (sys_path(path) != NULL || dev_path(path) != NULL) {
        return FS_READ_ONLY;
    }
    const char *rest;
    struct p9 *m = p9_resolve(path, &rest);
    return m != NULL ? p9_remove(m, rest) : ext2_remove(path);
}

int fs_format(const char *path, const char *label, const bool force, const u32 serial) {
    NORMALIZE(path, normalized_path);
    MOUNT_POINT(path, mount_point);
    return is_virtual(path) ? FS_INVALID : ext2_format(path, label, force, serial);
}

int fs_set_label(const char *path, const char *label, const u32 serial) {
    NORMALIZE(path, normalized_path);
    MOUNT_POINT(path, mount_point);
    return is_virtual(path) ? FS_INVALID : ext2_set_label(path, label, serial);
}

/* ---------------------------------------------------------------------------------------- */
/* Open files */

long fs_read(const fs_fd fd, void *dst, const size_t len) {
    power_check();
    return fd >= DEV_FD_BASE ? devfs_read(fd - DEV_FD_BASE, dst, len)
        : fd >= RAW_FD_BASE ? ext2_raw_read(fd - RAW_FD_BASE, dst, len)
        : fd >= P9_FD_BASE ? p9_read(fd - P9_FD_BASE, dst, len)
        : fd >= SYS_FD_BASE ? sysfs_read(fd - SYS_FD_BASE, dst, len)
        : ext2_read(fd, dst, len);
}

long fs_write(const fs_fd fd, const void *src, const size_t len) {
    power_check();
    return fd >= DEV_FD_BASE ? devfs_write(fd - DEV_FD_BASE, src, len)
        : fd >= RAW_FD_BASE ? ext2_raw_write(fd - RAW_FD_BASE, src, len)
        : fd >= P9_FD_BASE ? p9_write(fd - P9_FD_BASE, src, len)
        : fd >= SYS_FD_BASE ? FS_READ_ONLY
        : ext2_write(fd, src, len);
}

long fs_seek(const fs_fd fd, const long offset, const int whence) {
    power_check();
    return fd >= DEV_FD_BASE ? devfs_seek(fd - DEV_FD_BASE, offset, whence)
        : fd >= RAW_FD_BASE ? ext2_raw_seek(fd - RAW_FD_BASE, offset, whence)
        : fd >= P9_FD_BASE ? p9_seek(fd - P9_FD_BASE, offset, whence)
        : fd >= SYS_FD_BASE ? sysfs_seek(fd - SYS_FD_BASE, offset, whence)
        : ext2_seek(fd, offset, whence);
}

int fs_close(const fs_fd fd) {
    power_check();
    return fd >= DEV_FD_BASE ? devfs_close(fd - DEV_FD_BASE)
        : fd >= RAW_FD_BASE ? ext2_raw_close(fd - RAW_FD_BASE)
        : fd >= P9_FD_BASE ? p9_close(fd - P9_FD_BASE)
        : fd >= SYS_FD_BASE ? sysfs_close(fd - SYS_FD_BASE)
        : ext2_close(fd);
}
