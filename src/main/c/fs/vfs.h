/* SPDX-License-Identifier: MIT */
/* The VFS's kernel-internal API, see vfs.c. Backends are ext2.h, p9.h and sysfs.h. */
#ifndef VFS_H
#define VFS_H

#include "fs.h"

void fs_mount_all(void);
/* Queues a medium event for fs_take_medium_event. */
void fs_medium_event(const char *path, const char *what);

int fs_listing_add(struct fs_listing *out, const char *name, size_t len, bool is_dir);
/* Like fs_listing_add, unless the name is already listed. */
int fs_listing_add_unique(struct fs_listing *out, const char *name, size_t len, bool is_dir);

typedef void (*fs_mounts_fn)(void *ctx, const struct fs_mount *mount);

#endif
