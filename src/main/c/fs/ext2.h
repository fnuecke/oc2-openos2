/* SPDX-License-Identifier: MIT */
/* The ext2 backend and mkfs. */
#ifndef EXT2_H
#define EXT2_H

#include "vfs.h"

#define EXT2_MAGIC 0xEF53
#define ROOT_INODE 2
#define FT_DIR 2
#define INCOMPAT_FILETYPE 0x2
#define RO_COMPAT_SPARSE_SUPER 0x1
#define VOLUME_NAME_OFFSET 120 /* s_volume_name, in the superblock */

/* ---------------------------------------------------------------------------------------- */
/* ext2.c: / and /mnt/<label> or /mnt/<device index>, file descriptors 0 to 15 */

typedef int ext2_fd;
typedef int ext2_raw_fd; /* see fs_open_raw */

void ext2_mount_all(void);
void ext2_unmount_all(void);
void ext2_poll_media(void);
void ext2_mounts(fs_mounts_fn fn, void *ctx);
void ext2_mount_names(struct fs_listing *out);

int ext2_info(const char *path, u64 *total, u64 *free);
int ext2_stat(const char *path, struct fs_stat *st);
int ext2_list(const char *path, struct fs_listing *out);
int ext2_label(const char *path, char *label);

ext2_fd ext2_open(const char *path, int flags);
ext2_raw_fd ext2_open_raw(const char *path, int flags, bool force, u32 serial);

int ext2_mkdir(const char *path);
int ext2_rename(const char *from, const char *to);
int ext2_remove(const char *path);
int ext2_format(const char *path, const char *label, bool force, u32 serial);
int ext2_set_label(const char *path, const char *label, u32 serial);

long ext2_read(ext2_fd fd, void *dst, size_t len);
long ext2_write(ext2_fd fd, const void *src, size_t len);
long ext2_seek(ext2_fd fd, long offset, int whence);
int ext2_close(ext2_fd fd);
long ext2_raw_read(ext2_raw_fd fd, void *dst, size_t len);
long ext2_raw_write(ext2_raw_fd fd, const void *src, size_t len);
long ext2_raw_seek(ext2_raw_fd fd, long offset, int whence);
int ext2_raw_close(ext2_raw_fd fd);

/* mkfs.c */

struct vblk;
int ext2_mkfs(struct vblk *dev, const char *label);

#endif
