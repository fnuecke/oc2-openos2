/* SPDX-License-Identifier: MIT */
/* The read-only /sys backend, paths relative to /sys. */
#ifndef SYSFS_H
#define SYSFS_H

#include "vfs.h"

typedef int sysfs_fd;

int sysfs_port(const char *name, size_t len);
int sysfs_stat(const char *path, struct fs_stat *st);
int sysfs_list(const char *path, struct fs_listing *out);
sysfs_fd sysfs_open(const char *path, int flags);

long sysfs_read(sysfs_fd fd, void *dst, size_t len);
long sysfs_seek(sysfs_fd fd, long offset, int whence);
int sysfs_close(sysfs_fd fd);

#endif
