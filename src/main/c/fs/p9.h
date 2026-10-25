/* SPDX-License-Identifier: MIT */
/* The 9P2000.L backend: /mnt/<mount tag>, paths relative to the mount. */
#ifndef P9_H
#define P9_H

#include "vfs.h"

struct p9;
typedef int p9_fd;

void p9_mount_all(void);
void p9_mounts(fs_mounts_fn fn, void *ctx);
void p9_names(struct fs_listing *out);

struct p9 *p9_resolve(const char *path, const char **rest);
int p9_info(struct p9 *m, u64 *total, u64 *free);
int p9_label(struct p9 *m, const char *path, char *label);
int p9_stat(struct p9 *m, const char *path, struct fs_stat *st);
int p9_list(struct p9 *m, const char *path, struct fs_listing *out);
p9_fd p9_open(struct p9 *m, const char *path, int flags);
int p9_mkdir(struct p9 *m, const char *path);
int p9_rename(struct p9 *m, const char *from, const char *to);
int p9_remove(struct p9 *m, const char *path);

long p9_read(p9_fd fd, void *dst, size_t len);
long p9_write(p9_fd fd, const void *src, size_t len);
long p9_seek(p9_fd fd, long offset, int whence);
int p9_close(p9_fd fd);

#endif
