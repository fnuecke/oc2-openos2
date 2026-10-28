/* SPDX-License-Identifier: MIT */
/*
 * /dev with Linux's names:
 *
 *   null, zero               read nothing or zeros, discard writes
 *   random, urandom          bytes from a generator mixed with entropy() on every read, not cryptographic
 *   vda, vdb, ...            the whole medium of each block device, through fs_open_raw, at any offset
 */

#include <stdio.h>
#include <string.h>
#include "drivers/virtio/virtio.h"
#include "devfs.h"
#include "ext2.h"
#include "internal.h"

enum { NUL, ZERO, RANDOM, DISK, DIR, NONE };

#define DISK_FD (RANDOM + 1) /* plus the raw handle */

static const struct {
    const char *name;
    int kind;
} devices[] = {
    {"null", NUL}, {"random", RANDOM}, {"urandom", RANDOM}, {"zero", ZERO},
};

static struct disk {
    int raw;
    int device;
    u64 pos;
} disks[EXT2_RAW_HANDLES];
static u8 sector[VBLK_SECTOR_SIZE];
static u64 random_state;

static int kind_of(const char *path, int *disk) {
    while (*path == '/') {
        path++;
    }
    if (*path == 0) {
        return DIR;
    }
    for (size_t i = 0; i < sizeof(devices) / sizeof(devices[0]); i++) {
        if (strcmp(path, devices[i].name) == 0) {
            return devices[i].kind;
        }
    }
    if (path[0] == 'v' && path[1] == 'd' && path[2] >= 'a' && path[2] - 'a' < vblk_count() && path[3] == 0) {
        *disk = path[2] - 'a';
        return DISK;
    }
    return NONE;
}

/* ---------------------------------------------------------------------------------------- */
/* Querying */

int devfs_mount_point(const char *path, char *mount, u32 *serial) {
    int disk;
    const int kind = kind_of(path, &disk);
    if (kind != DISK) {
        return kind == NONE ? FS_NOT_FOUND : FS_INVALID;
    }
    if (!ext2_mount_path(disk, mount, serial)) {
        return vblk_capacity(vblk_get(disk)) == 0 ? FS_NO_MEDIUM : FS_BUSY;
    }
    return 0;
}

int devfs_stat(const char *path, struct fs_stat *st) {
    int disk;
    const int kind = kind_of(path, &disk);
    if (kind == NONE) {
        return FS_NOT_FOUND;
    }
    *st = (struct fs_stat) {.is_dir = kind == DIR, .is_device = kind != DIR};
    return 0;
}

int devfs_list(const char *path, struct fs_listing *out) {
    int disk;
    const int kind = kind_of(path, &disk);
    if (kind != DIR) {
        return kind == NONE ? FS_NOT_FOUND : FS_NOT_DIR;
    }
    for (size_t i = 0; i < sizeof(devices) / sizeof(devices[0]); i++) {
        fs_listing_add(out, devices[i].name, strlen(devices[i].name), 0);
    }
    for (int i = 0; i < vblk_count(); i++) {
        const char name[] = {'v', 'd', (char) ('a' + i)};
        fs_listing_add(out, name, sizeof(name), 0);
    }
    return 0;
}

/* ---------------------------------------------------------------------------------------- */
/* Opening */

devfs_fd devfs_open(const char *path, const int flags) {
    int disk;
    const int kind = kind_of(path, &disk);
    if (kind <= RANDOM) {
        return kind;
    }
    if (kind != DISK) {
        return kind == DIR ? FS_IS_DIR : FS_NOT_FOUND;
    }
    if (flags & O_APPEND) {
        return FS_NO_SPACE;
    }
    char mount[EXT2_PATH_LENGTH];
    u32 serial;
    const int found = devfs_mount_point(path, mount, &serial);
    if (found != 0) {
        return found;
    }
    const int raw = ext2_open_raw(mount, flags & O_WRITE ? O_WRITE : O_READ, 0, serial);
    if (raw < 0) {
        return raw;
    }
    disks[raw] = (struct disk) {.raw = raw, .device = disk};
    return DISK_FD + raw;
}

/* ---------------------------------------------------------------------------------------- */
/* Open files */

static struct disk *disk_of(const devfs_fd fd) {
    return fd >= DISK_FD && fd < DISK_FD + EXT2_RAW_HANDLES ? &disks[fd - DISK_FD] : NULL;
}

static void fill_random(u8 *dst, const size_t len) {
    random_state = mix64(random_state ^ entropy());
    u64 word = 0;
    for (size_t i = 0; i < len; i++) {
        if (i % 8 == 0) {
            word = mix64(random_state);
            random_state += 0x9E3779B97F4A7C15ull;
        }
        dst[i] = (u8) (word >> (i % 8 * 8));
    }
}

static long disk_transfer(const int write, struct disk *d, u8 *data, const size_t len) {
    size_t done = 0;
    long result = 0;
    while (done < len) {
        const size_t offset = (size_t) ((d->pos + done) % VBLK_SECTOR_SIZE);
        const long start = (long) (d->pos + done - offset);
        result = ext2_raw_seek(d->raw, start, SEEK_SET);
        if (result >= 0 && offset == 0 && len - done >= VBLK_SECTOR_SIZE) {
            const size_t whole = (len - done) & ~(size_t) (VBLK_SECTOR_SIZE - 1);
            result = write ? ext2_raw_write(d->raw, data + done, whole) : ext2_raw_read(d->raw, data + done, whole);
            if (result > 0) {
                done += (size_t) result;
            }
            if (result != (long) whole) {
                break;
            }
            continue;
        }
        if (result >= 0) {
            result = ext2_raw_read(d->raw, sector, VBLK_SECTOR_SIZE);
        }
        if (result <= 0) {
            result = result == 0 && write ? FS_NO_SPACE : result;
            break;
        }
        const size_t count = VBLK_SECTOR_SIZE - offset < len - done ? VBLK_SECTOR_SIZE - offset : len - done;
        if (write) {
            memcpy(sector + offset, data + done, count);
            result = ext2_raw_seek(d->raw, start, SEEK_SET);
            if (result >= 0) {
                result = ext2_raw_write(d->raw, sector, VBLK_SECTOR_SIZE);
            }
            if (result < 0) {
                break;
            }
        } else {
            memcpy(data + done, sector + offset, count);
        }
        done += count;
    }
    d->pos += done;
    return done > 0 || len == 0 ? (long) done : result;
}

long devfs_read(const devfs_fd fd, void *dst, const size_t len) {
    struct disk *d = disk_of(fd);
    if (d != NULL) {
        return disk_transfer(0, d, dst, len);
    }
    if (fd == ZERO) {
        memset(dst, 0, len);
    } else if (fd == RANDOM) {
        fill_random(dst, len);
    } else if (fd != NUL) {
        return FS_INVALID;
    }
    return fd == NUL ? 0 : (long) len;
}

long devfs_write(const devfs_fd fd, const void *src, const size_t len) {
    struct disk *d = disk_of(fd);
    return d != NULL ? disk_transfer(1, d, (void *) src, len) : fd >= 0 && fd <= RANDOM ? (long) len : FS_INVALID;
}

long devfs_seek(const devfs_fd fd, const long offset, const int whence) {
    struct disk *d = disk_of(fd);
    if (d == NULL) {
        return fd >= 0 && fd <= RANDOM ? 0 : FS_INVALID;
    }
    const long size = (long) (vblk_capacity(vblk_get(d->device)) * VBLK_SECTOR_SIZE);
    const long base = whence == SEEK_SET ? 0 : whence == SEEK_CUR ? (long) d->pos : size;
    if (base + offset < 0) {
        return FS_INVALID;
    }
    d->pos = (u64) (base + offset);
    return (long) d->pos;
}

int devfs_close(const devfs_fd fd) {
    struct disk *d = disk_of(fd);
    if (d == NULL) {
        return fd >= 0 && fd <= RANDOM ? 0 : FS_INVALID;
    }
    return ext2_raw_close(d->raw);
}
