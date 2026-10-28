/* SPDX-License-Identifier: MIT */
#ifndef DEVFS_H
#define DEVFS_H

#include "vfs.h"

typedef int devfs_fd;

int devfs_mount_point(const char *path, char *mount, u32 *serial);
int devfs_stat(const char *path, struct fs_stat *st);
int devfs_list(const char *path, struct fs_listing *out);
devfs_fd devfs_open(const char *path, int flags);

long devfs_read(devfs_fd fd, void *dst, size_t len);
long devfs_write(devfs_fd fd, const void *src, size_t len);
long devfs_seek(devfs_fd fd, long offset, int whence);
int devfs_close(devfs_fd fd);

#endif
