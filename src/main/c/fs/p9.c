/* SPDX-License-Identifier: MIT */
/*
 * 9P2000.L client over virtio-9p, one request in flight per mount. File data is not copied: the
 * reply header and the caller's buffer are consecutive device-writable descriptors, and the device
 * writes the reply across them in order.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "drivers/plic.h"
#include "drivers/virtio/virtio.h"
#include "internal.h"
#include "p9.h"

#define VIRTIO_ID_9P 9
#define F_MOUNT_TAG (1u << 0)

#define TLERROR 6
#define TSTATFS 8
#define TLOPEN 12
#define TLCREATE 14
#define TGETATTR 24
#define TREADDIR 40
#define TMKDIR 72
#define TRENAMEAT 74
#define TUNLINKAT 76
#define TVERSION 100
#define TATTACH 104
#define TWALK 110
#define TREAD 116
#define TWRITE 118
#define TCLUNK 120

#define NOTAG 0xFFFF
#define NOFID 0xFFFFFFFFu
#define TAG 1
#define ROOT_FID 0

#define L_O_RDONLY 0
#define L_O_WRONLY 1
#define L_O_RDWR 2
#define L_O_TRUNC 0x200
#define L_O_DIRECTORY 0x10000
#define GETATTR_BASIC 0x7FFull
#define DT_DIR 4

#define HEADER 7
#define IO_REPLY 11
#define IO_REQUEST 23
#define MSIZE 8192
#define CHUNK (MSIZE - 64)
#define TX_SIZE 512
#define RX_SIZE 256
#define DIR_BUFFER 1024
#define QUEUE_SIZE 4
#define MAX_MOUNTS 2
#define MAX_FILES 8
#define MAX_WALK 16
#define TIMEOUT_SECONDS 10

#define E_NOENT 2
#define E_EXIST 17
#define E_NOTDIR 20
#define E_ISDIR 21
#define E_NOSPC 28
#define E_ROFS 30
#define E_NOTEMPTY 39

struct p9 {
    u64 base;
    char path[48]; /* "/mnt/<tag>" */
    int dead; /* a request timed out, its buffers may still be written */
    bool read_only;
    struct virtq queue;
    u32 fids; /* bit set per fid in use, bit 0 is the root */
    u8 tx[TX_SIZE];
    u8 rx[RX_SIZE];
};

struct file {
    struct p9 *mount;
    u32 fid;
    u64 pos;
    int flags;
};

struct name {
    const char *text;
    u32 len;
};

static struct p9 *mounts[MAX_MOUNTS];
static int mount_count;
static struct file files[MAX_FILES];
static u8 dir_buffer[DIR_BUFFER];

/* ---------------------------------------------------------------------------------------- */
/* Messages */

static u8 *put16(u8 *p, const u16 value) {
    memcpy(p, &value, 2);
    return p + 2;
}

static u8 *put32(u8 *p, const u32 value) {
    memcpy(p, &value, 4);
    return p + 4;
}

static u8 *put64(u8 *p, const u64 value) {
    memcpy(p, &value, 8);
    return p + 8;
}

static u8 *put_string(u8 *p, const char *s, const u32 len) {
    p = put16(p, (u16) len);
    memcpy(p, s, len);
    return p + len;
}

static u32 get32(const u8 *p) {
    u32 value;
    memcpy(&value, p, 4);
    return value;
}

static u64 get64(const u8 *p) {
    u64 value;
    memcpy(&value, p, 8);
    return value;
}

static u8 *begin(struct p9 *m, const u8 type) {
    m->tx[4] = type;
    put16(m->tx + 5, type == TVERSION ? NOTAG : TAG);
    return m->tx + HEADER;
}

static int map_errno(const u32 number) {
    switch (number) {
        case E_NOENT: return FS_NOT_FOUND;
        case E_EXIST: return FS_EXISTS;
        case E_NOTDIR: return FS_NOT_DIR;
        case E_ISDIR: return FS_IS_DIR;
        case E_NOSPC: return FS_NO_SPACE;
        case E_ROFS: return FS_READ_ONLY;
        case E_NOTEMPTY: return FS_NOT_EMPTY;
        default: return FS_IO;
    }
}

static bool has_used(void *ctx) {
    const struct virtq *q = ctx;
    return *(volatile u16 *) &q->used->idx != q->last_used;
}

static int transact(struct p9 *m, const u8 *end, const void *out, const u32 out_len,
                    const u32 reply, void *in, const u32 in_len) {
    if (m->dead) {
        return FS_IO;
    }
    const u32 length = (u32) (end - m->tx) + out_len;
    put32(m->tx, length);
    struct virtq_buf bufs[4];
    int count = 0;
    bufs[count++] = (struct virtq_buf) {m->tx, (u32) (end - m->tx)};
    if (out_len > 0) {
        bufs[count++] = (struct virtq_buf) {(void *) out, out_len};
    }
    const int out_count = count;
    bufs[count++] = (struct virtq_buf) {m->rx, reply};
    if (in_len > 0) {
        bufs[count++] = (struct virtq_buf) {in, in_len};
    }
    const u8 type = m->tx[4];
    m->rx[4] = 0;
    if (virtq_add(&m->queue, bufs, out_count, count - out_count) < 0) {
        return FS_IO;
    }
    virtq_notify(&m->queue);

    const u64 deadline = time_now() + TIMEOUT_SECONDS * board.timebase;
    while (!has_used(&m->queue)) {
        if (time_now() >= deadline) {
            virtio_reset(m->base);
            m->dead = 1;
            return FS_IO;
        }
        wait_until(has_used, &m->queue, deadline);
    }
    virtq_next_used(&m->queue, NULL);

    if (m->rx[4] == TLERROR + 1) {
        return map_errno(get32(m->rx + HEADER));
    }
    return m->rx[4] == type + 1 ? 0 : FS_IO;
}

static long alloc_fid(struct p9 *m) {
    for (u32 fid = 1; fid < 32; fid++) {
        if (!(m->fids & (1u << fid))) {
            m->fids |= 1u << fid;
            return fid;
        }
    }
    return FS_TOO_MANY;
}

static void clunk(struct p9 *m, const u32 fid) {
    u8 *p = put32(begin(m, TCLUNK), fid);
    transact(m, p, NULL, 0, HEADER, NULL, 0);
    m->fids &= ~(1u << fid);
}

static int split(const char *path, struct name *names, int *count) {
    *count = 0;
    while (*path) {
        while (*path == '/') {
            path++;
        }
        const char *start = path;
        while (*path && *path != '/') {
            path++;
        }
        const u32 len = (u32) (path - start);
        if (len == 0 || (len == 1 && start[0] == '.')) {
            continue;
        }
        if (*count == MAX_WALK || len > 255) {
            return FS_INVALID;
        }
        names[(*count)++] = (struct name) {start, len};
    }
    return 0;
}

static int walk(struct p9 *m, const u32 newfid, const struct name *names, const int count) {
    u8 *p = begin(m, TWALK);
    p = put32(p, ROOT_FID);
    p = put32(p, newfid);
    p = put16(p, (u16) count);
    for (int i = 0; i < count; i++) {
        if (p + 2 + names[i].len > m->tx + TX_SIZE) {
            return FS_INVALID;
        }
        p = put_string(p, names[i].text, names[i].len);
    }
    const int result = transact(m, p, NULL, 0, RX_SIZE, NULL, 0);
    if (result != 0) {
        return result;
    }
    return (m->rx[HEADER] | m->rx[HEADER + 1] << 8) == count ? 0 : FS_NOT_FOUND;
}

static long walk_path(struct p9 *m, const char *path, struct name *last) {
    struct name names[MAX_WALK];
    int count;
    int result = split(path, names, &count);
    if (result != 0) {
        return result;
    }
    if (last != NULL) {
        if (count == 0) {
            return FS_INVALID;
        }
        *last = names[--count];
    }
    const long fid = alloc_fid(m);
    if (fid < 0) {
        return fid;
    }
    result = walk(m, (u32) fid, names, count);
    if (result != 0) {
        m->fids &= ~(1u << fid);
        return result;
    }
    return fid;
}

static int getattr(struct p9 *m, const u32 fid, struct fs_stat *st) {
    u8 *p = put32(begin(m, TGETATTR), fid);
    p = put64(p, GETATTR_BASIC);
    const int result = transact(m, p, NULL, 0, RX_SIZE, NULL, 0);
    if (result != 0) {
        return result;
    }
    const u8 *r = m->rx + HEADER;
    st->is_dir = (get32(r + 21) & 0xF000) == 0x4000;
    st->size = get64(r + 49);
    st->mtime = (u32) get64(r + 89);
    return 0;
}

/* ---------------------------------------------------------------------------------------- */
/* Mounting */

static int mount_device(const u64 base, const u32 irq) {
    if (virtio_device_id(base) != VIRTIO_ID_9P || mount_count == MAX_MOUNTS) {
        return -1;
    }
    const long features = virtio_begin(base, F_MOUNT_TAG);
    if (features < 0 || !(features & F_MOUNT_TAG)) {
        virtio_reset(base);
        return -1;
    }
    char tag[32];
    const u32 tag_length = virtio_config8(base, 0) | (u32) virtio_config8(base, 1) << 8;
    if (tag_length == 0 || tag_length >= sizeof(tag)) {
        virtio_reset(base);
        return -1;
    }
    for (u32 i = 0; i < tag_length; i++) {
        tag[i] = (char) virtio_config8(base, 2 + i);
        const char c = tag[i];
        if (!((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') || c == '.' || c == '_' || c == '-')) {
            virtio_reset(base);
            return -1;
        }
    }
    tag[tag_length] = 0;
    if (strspn(tag, "0123456789") == tag_length) {
        virtio_reset(base);
        return -1;
    }

    struct p9 *m = calloc(1, sizeof(struct p9));
    if (m == NULL || virtq_init(&m->queue, base, 0, QUEUE_SIZE) != 0) {
        virtio_reset(base);
        free(m);
        return -1;
    }
    m->base = base;
    strcpy(m->path, "/mnt/");
    strcat(m->path, tag);
    virtio_ready(base);
    plic_enable(irq);

    u8 *p = put32(begin(m, TVERSION), MSIZE);
    p = put_string(p, "9P2000.L", 8);
    int result = transact(m, p, NULL, 0, RX_SIZE, NULL, 0);
    if (result == 0 && (get32(m->rx + HEADER) < MSIZE || memcmp(m->rx + HEADER + 4, "\x08\x00" "9P2000.L", 10) != 0)) {
        result = FS_INVALID;
    }
    if (result == 0) {
        p = put32(begin(m, TATTACH), ROOT_FID);
        p = put32(p, NOFID);
        p = put_string(p, "", 0);
        p = put_string(p, "", 0);
        p = put32(p, 0);
        result = transact(m, p, NULL, 0, RX_SIZE, NULL, 0);
    }
    if (result != 0) {
        virtio_reset(base);
        free(m);
        return -1;
    }
    m->fids = 1u << ROOT_FID;
    struct fs_stat root;
    if (getattr(m, ROOT_FID, &root) == 0) {
        m->read_only = (get32(m->rx + HEADER + 21) & 0222) == 0;
    }
    mounts[mount_count++] = m;
    return 0;
}

void p9_mount_all(void) {
    for (int i = 0; i < board.virtio_count; i++) {
        mount_device(board.virtio[i], board.virtio_irq[i]);
    }
}

void p9_mounts(const fs_mounts_fn fn, void *ctx) {
    for (int i = 0; i < mount_count; i++) {
        struct fs_mount mount = {.path = mounts[i]->path, .label = mounts[i]->path + 5, .index = -1,
            .medium = FS_MEDIUM_9P, .writable = !mounts[i]->read_only};
        p9_info(mounts[i], &mount.total, &mount.free);
        fn(ctx, &mount);
    }
}

void p9_names(struct fs_listing *out) {
    for (int i = 0; i < mount_count; i++) {
        const char *name = mounts[i]->path + 5;
        fs_listing_add_unique(out, name, strlen(name), 1);
    }
}

/* ---------------------------------------------------------------------------------------- */
/* Querying */

struct p9 *p9_resolve(const char *path, const char **rest) {
    while (*path == '/') {
        path++;
    }
    for (int i = 0; i < mount_count; i++) {
        const char *mount = mounts[i]->path + 1;
        const size_t len = strlen(mount);
        if (strncmp(path, mount, len) == 0 && (path[len] == 0 || path[len] == '/')) {
            *rest = path + len;
            return mounts[i];
        }
    }
    return NULL;
}

int p9_info(struct p9 *m, u64 *total, u64 *free) {
    u8 *p = put32(begin(m, TSTATFS), ROOT_FID);
    const int result = transact(m, p, NULL, 0, RX_SIZE, NULL, 0);
    if (result != 0) {
        return result;
    }
    const u64 block_size = get32(m->rx + HEADER + 4);
    *total = get64(m->rx + HEADER + 8) * block_size;
    *free = get64(m->rx + HEADER + 16) * block_size;
    return 0;
}

int p9_label(struct p9 *m, const char *path, char *label) {
    if (*path != 0 && strcmp(path, "/") != 0) {
        return FS_INVALID;
    }
    snprintf(label, FS_LABEL_LENGTH + 1, "%s", m->path + 5);
    return 0;
}

int p9_stat(struct p9 *m, const char *path, struct fs_stat *st) {
    const long fid = walk_path(m, path, NULL);
    if (fid < 0) {
        return (int) fid;
    }
    const int result = getattr(m, (u32) fid, st);
    clunk(m, (u32) fid);
    return result;
}

int p9_list(struct p9 *m, const char *path, struct fs_listing *out) {
    const long fid = walk_path(m, path, NULL);
    if (fid < 0) {
        return (int) fid;
    }
    u8 *p = put32(begin(m, TLOPEN), (u32) fid);
    p = put32(p, L_O_RDONLY | L_O_DIRECTORY);
    int result = transact(m, p, NULL, 0, RX_SIZE, NULL, 0);
    u64 offset = 0;
    while (result == 0) {
        p = put32(begin(m, TREADDIR), (u32) fid);
        p = put64(p, offset);
        p = put32(p, DIR_BUFFER);
        result = transact(m, p, NULL, 0, IO_REPLY, dir_buffer, DIR_BUFFER);
        u32 count = result == 0 ? get32(m->rx + HEADER) : 0;
        if (count == 0) {
            break;
        }
        if (count > DIR_BUFFER) {
            count = DIR_BUFFER;
        }
        for (u32 at = 0; at + 24 <= count && result == 0;) {
            const u8 *e = dir_buffer + at;
            const u32 len = e[22] | (u32) e[23] << 8;
            if (at + 24 + len > count) {
                break;
            }
            offset = get64(e + 13);
            const char *name = (const char *) e + 24;
            const int dot = len == 1 && name[0] == '.';
            const int dotdot = len == 2 && name[0] == '.' && name[1] == '.';
            if (!dot && !dotdot) {
                result = fs_listing_add(out, name, len, e[21] == DT_DIR);
            }
            at += 24 + len;
        }
    }
    clunk(m, (u32) fid);
    return result;
}

/* ---------------------------------------------------------------------------------------- */
/* Opening */

p9_fd p9_open(struct p9 *m, const char *path, const int flags) {
    if (m->read_only && (flags & (O_WRITE | O_CREATE | O_TRUNCATE | O_APPEND))) {
        return FS_READ_ONLY;
    }
    int fd = -1;
    for (int i = 0; i < MAX_FILES; i++) {
        if (files[i].mount == NULL) {
            fd = i;
            break;
        }
    }
    if (fd < 0) {
        return FS_TOO_MANY;
    }
    const size_t length = strlen(path);
    if (length > 0 && path[length - 1] == '/') {
        return FS_IS_DIR;
    }
    u32 mode = (flags & O_READ) && (flags & O_WRITE) ? L_O_RDWR : (flags & O_WRITE) ? L_O_WRONLY : L_O_RDONLY;
    if ((flags & O_TRUNCATE) && (flags & O_WRITE)) {
        mode |= L_O_TRUNC;
    }

    long fid = walk_path(m, path, NULL);
    int result;
    if (fid == FS_NOT_FOUND && (flags & O_CREATE)) {
        struct name name;
        fid = walk_path(m, path, &name);
        if (fid < 0) {
            return (int) fid;
        }
        u8 *p = put32(begin(m, TLCREATE), (u32) fid);
        p = put_string(p, name.text, name.len);
        p = put32(p, mode);
        p = put32(p, 0644);
        p = put32(p, 0);
        result = transact(m, p, NULL, 0, RX_SIZE, NULL, 0);
    } else {
        if (fid < 0) {
            return (int) fid;
        }
        struct fs_stat st;
        result = getattr(m, (u32) fid, &st);
        if (result == 0 && st.is_dir) {
            result = FS_IS_DIR;
        }
        if (result == 0) {
            u8 *p = put32(begin(m, TLOPEN), (u32) fid);
            p = put32(p, mode);
            result = transact(m, p, NULL, 0, RX_SIZE, NULL, 0);
        }
    }
    if (result != 0) {
        clunk(m, (u32) fid);
        return result;
    }
    files[fd] = (struct file) {m, (u32) fid, 0, flags};
    return fd;
}

/* ---------------------------------------------------------------------------------------- */
/* Modifying */

int p9_mkdir(struct p9 *m, const char *path) {
    if (m->read_only) {
        return FS_READ_ONLY;
    }
    struct name name;
    const long fid = walk_path(m, path, &name);
    if (fid < 0) {
        return (int) fid;
    }
    u8 *p = put32(begin(m, TMKDIR), (u32) fid);
    p = put_string(p, name.text, name.len);
    p = put32(p, 0755);
    p = put32(p, 0);
    const int result = transact(m, p, NULL, 0, RX_SIZE, NULL, 0);
    clunk(m, (u32) fid);
    return result;
}

int p9_rename(struct p9 *m, const char *from, const char *to) {
    if (m->read_only) {
        return FS_READ_ONLY;
    }
    struct name from_name, to_name;
    const long source = walk_path(m, from, NULL);
    if (source < 0) {
        return (int) source;
    }
    clunk(m, (u32) source);
    const long from_fid = walk_path(m, from, &from_name);
    if (from_fid < 0) {
        return (int) from_fid;
    }
    const long to_fid = walk_path(m, to, &to_name);
    if (to_fid < 0) {
        clunk(m, (u32) from_fid);
        return (int) to_fid;
    }
    u8 *p = put32(begin(m, TUNLINKAT), (u32) to_fid);
    p = put_string(p, to_name.text, to_name.len);
    p = put32(p, 0);
    transact(m, p, NULL, 0, RX_SIZE, NULL, 0);
    p = put32(begin(m, TRENAMEAT), (u32) from_fid);
    p = put_string(p, from_name.text, from_name.len);
    p = put32(p, (u32) to_fid);
    p = put_string(p, to_name.text, to_name.len);
    const int result = transact(m, p, NULL, 0, RX_SIZE, NULL, 0);
    clunk(m, (u32) from_fid);
    clunk(m, (u32) to_fid);
    return result;
}

int p9_remove(struct p9 *m, const char *path) {
    if (m->read_only) {
        return FS_READ_ONLY;
    }
    struct name name;
    const long fid = walk_path(m, path, &name);
    if (fid < 0) {
        return (int) fid;
    }
    u8 *p = put32(begin(m, TUNLINKAT), (u32) fid);
    p = put_string(p, name.text, name.len);
    p = put32(p, 0);
    const int result = transact(m, p, NULL, 0, RX_SIZE, NULL, 0);
    clunk(m, (u32) fid);
    return result;
}

/* ---------------------------------------------------------------------------------------- */
/* Open files */

static struct file *get_file(const int fd) {
    return fd >= 0 && fd < MAX_FILES && files[fd].mount != NULL ? &files[fd] : NULL;
}

long p9_read(const p9_fd fd, void *dst, const size_t len) {
    struct file *f = get_file(fd);
    if (f == NULL || !(f->flags & O_READ)) {
        return FS_INVALID;
    }
    size_t done = 0;
    while (done < len) {
        const u32 want = len - done > CHUNK ? CHUNK : (u32) (len - done);
        u8 *p = put32(begin(f->mount, TREAD), f->fid);
        p = put64(p, f->pos);
        p = put32(p, want);
        const int result = transact(f->mount, p, NULL, 0, IO_REPLY, (u8 *) dst + done, want);
        if (result != 0) {
            return done > 0 ? (long) done : result;
        }
        u32 got = get32(f->mount->rx + HEADER);
        if (got > want) {
            got = 0;
        }
        done += got;
        f->pos += got;
        if (got < want) {
            break;
        }
    }
    return (long) done;
}

long p9_write(const p9_fd fd, const void *src, const size_t len) {
    struct file *f = get_file(fd);
    if (f == NULL || !(f->flags & O_WRITE)) {
        return FS_INVALID;
    }
    if (f->flags & O_APPEND) {
        struct fs_stat st;
        const int result = getattr(f->mount, f->fid, &st);
        if (result != 0) {
            return result;
        }
        f->pos = st.size;
    }
    size_t done = 0;
    while (done < len) {
        const u32 want = len - done > CHUNK ? CHUNK : (u32) (len - done);
        u8 *p = put32(begin(f->mount, TWRITE), f->fid);
        p = put64(p, f->pos);
        p = put32(p, want);
        const int result = transact(f->mount, p, (const u8 *) src + done, want, IO_REPLY, NULL, 0);
        if (result != 0) {
            return done > 0 ? (long) done : result;
        }
        const u32 wrote = get32(f->mount->rx + HEADER);
        done += wrote;
        f->pos += wrote;
        if (wrote == 0) {
            break;
        }
    }
    return (long) done;
}

long p9_seek(const p9_fd fd, const long offset, const int whence) {
    struct file *f = get_file(fd);
    if (f == NULL) {
        return FS_INVALID;
    }
    long base = whence == 1 ? (long) f->pos : 0;
    if (whence == 2) {
            struct fs_stat st;
        const int result = getattr(f->mount, f->fid, &st);
        if (result != 0) {
            return result;
        }
        base = (long) st.size;
    }
    if (base + offset < 0) {
        return FS_INVALID;
    }
    f->pos = (u64) (base + offset);
    return (long) f->pos;
}

int p9_close(const p9_fd fd) {
    struct file *f = get_file(fd);
    if (f == NULL) {
        return FS_INVALID;
    }
    clunk(f->mount, f->fid);
    f->mount = NULL;
    return 0;
}
