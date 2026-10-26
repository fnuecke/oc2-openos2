/* SPDX-License-Identifier: MIT */
#ifndef FS_H
#define FS_H

#include "kernel.h"

/* ext2 at /, /mnt/<label> and /mnt/<device index>, 9p at /mnt/<mount tag>, a read-only /sys */

/* ---------------------------------------------------------------------------------------- */
/* Errors, flags and types */

#define FS_EXISTS (-2)
#define FS_NOT_FOUND (-3)
#define FS_NOT_DIR (-4)
#define FS_IS_DIR (-5)
#define FS_NOT_EMPTY (-6)
#define FS_NO_SPACE (-7)
#define FS_READ_ONLY (-8)
#define FS_INVALID (-9)
#define FS_IO (-10)
#define FS_TOO_MANY (-11)
#define FS_CHANGED (-12)
#define FS_BUSY (-13)
#define FS_UNFORMATTED (-14)
#define FS_UNSUPPORTED (-15)
#define FS_IS_ROOT (-16)
#define FS_NOT_BLANK (-17)

#define O_READ 1
#define O_WRITE 2
#define O_CREATE 4
#define O_TRUNCATE 8
#define O_APPEND 16

#define FS_PATH_LENGTH 512
#define FS_LABEL_LENGTH 16

struct fs_stat {
    u64 size;
    u32 mtime;
    bool is_dir;
};

/* Directory entries collected by fs_list: per entry a byte is_dir, a byte name length, the name. */
struct fs_listing {
    u8 *data;
    size_t length, capacity;
};

typedef int fs_fd;

/* ---------------------------------------------------------------------------------------- */
/* Mounts and media */

#define FS_MEDIUM_UNKNOWN 0 /* other data */
#define FS_MEDIUM_BLANK 1
#define FS_MEDIUM_UNSUPPORTED 2 /* ext2/3/4 this kernel cannot use: unknown features, impossible geometry */
#define FS_MEDIUM_EXT2 3
#define FS_MEDIUM_9P 4

struct fs_mount {
    const char *path;
    const char *label; /* "" if none; for 9p the mount tag */
    int index; /* of the block device, -1 for 9p */
    int medium; /* FS_MEDIUM_* */
    u32 serial; /* new for every mount, also when the same medium is mounted again (e.g. a floppy
                 * reinserted), so a serial taken before a confirmation fails after any remount */
    u64 total, free; /* in bytes */
    bool writable;
};

const char *fs_error(int code);
/* Rewrites path lexically: one leading "/", no "." or empty names, ".." removes the name before it. */
int fs_normalize(const char *path, char *out);
void fs_mounts(void (*fn)(void *ctx, const struct fs_mount *mount), void *ctx);
/* Marks writable file systems as cleanly unmounted, before power off or reboot. */
void fs_unmount_all(void);
/*
 * Detaches the drive mounted at `path`: marks its file system clean and stops using the medium
 * until it is changed. Fails like fs_format for / and while files are open on it.
 */
int fs_unmount(const char *path, u32 serial);
/* Looks for medium changes. Every file system call also does. */
void fs_poll_media(void);
/*
 * Takes the oldest medium event: the mount at `path` was "added", "removed" or "changed".
 * Returns 0 if there is none.
 */
int fs_take_medium_event(char *path, size_t size, const char **what);
bool fs_medium_event_pending(void);

/* ---------------------------------------------------------------------------------------- */
/* Querying */

int fs_info(const char *path, u64 *total, u64 *free);
int fs_stat(const char *path, struct fs_stat *st);
/* Lists a directory into `out`, which starts zeroed, without "." and "..". The caller frees
 * out->data, also on errors. */
int fs_list(const char *path, struct fs_listing *out);
/*
 * Copies the label of the mount at `path` into `label`, FS_LABEL_LENGTH + 1 bytes. For 9p mounts
 * it is the mount tag, cut to FS_LABEL_LENGTH.
 */
int fs_label(const char *path, char *label);

/* ---------------------------------------------------------------------------------------- */
/* Opening */

fs_fd fs_open(const char *path, int flags);
/*
 * Opens the whole medium mounted at `path` as a file of its capacity, for fs_read, fs_write, fs_seek
 * and fs_close. Offsets and lengths must be multiples of 512 (FS_INVALID). `flags` is O_READ or
 * O_WRITE. Reading needs no file open for writing on the mount, and until it is closed, nothing on
 * the mount can be written (FS_BUSY). Writing checks like fs_format and detaches the mount until it
 * is closed (FS_BUSY), which then mounts the medium again.
 */
fs_fd fs_open_raw(const char *path, int flags, bool force, u32 serial);

/* ---------------------------------------------------------------------------------------- */
/* Modifying */

int fs_mkdir(const char *path);
int fs_rename(const char *from, const char *to);
int fs_remove(const char *path);
/*
 * Writes a new, empty ext2 file system on the medium mounted at `path`, which must be a mount point
 * other than / (FS_IS_ROOT) without open files. A NULL label keeps the current one, "" clears it.
 * Media that are not FS_MEDIUM_BLANK, and ext2 mounted read-only for its features, are only
 * overwritten with `force` (else FS_NOT_BLANK or FS_READ_ONLY). A `serial`
 * other than 0 must be the mount's, see struct fs_mount, or this fails with FS_CHANGED. Afterwards
 * the medium is mounted again, at /mnt/<label> if the label allows, see fs_set_label.
 */
int fs_format(const char *path, const char *label, bool force, u32 serial);
/*
 * Sets the label of the ext2 mount at `path`, "" clears it. Labels are 1 to FS_LABEL_LENGTH of
 * A-Z, a-z, 0-9, ".", "_" and "-", not only digits nor "." or "..". The mount then moves to
 * /mnt/<label>, or to /mnt/<device index> without a label; / stays. Fails with FS_EXISTS if
 * another mount or a directory in the root file system's /mnt uses the name, with FS_BUSY if files
 * are open on a mount that would move, and with FS_CHANGED for another `serial`, see fs_format.
 */
int fs_set_label(const char *path, const char *label, u32 serial);

/* ---------------------------------------------------------------------------------------- */
/* Open files */

long fs_read(fs_fd fd, void *dst, size_t len);
long fs_write(fs_fd fd, const void *src, size_t len);
long fs_seek(fs_fd fd, long offset, int whence);
int fs_close(fs_fd fd);

#endif
