/* SPDX-License-Identifier: MIT */
#ifndef VIRTIO_H
#define VIRTIO_H

#include "kernel.h"

#define VIRTIO_ID_BLOCK 2
#define VIRTIO_ID_CONSOLE 3

/* ---------------------------------------------------------------------------------------- */
/* virtio.c: virtio-mmio transport and split virtqueues */

#define VIRTIO_REG_CONFIG 0x100

void virtio_interrupt(u32 irq);

struct virtq_desc {
    u64 addr;
    u32 len;
    u16 flags;
    u16 next;
};

struct virtq_avail {
    u16 flags;
    u16 idx;
    u16 ring[];
};

struct virtq_used_elem {
    u32 id;
    u32 len;
};

struct virtq_used {
    u16 flags;
    u16 idx;
    struct virtq_used_elem ring[];
};

struct virtq {
    u64 base;
    u16 index;
    u16 size;
    u16 free_head;
    u16 free_count;
    u16 last_used;
    struct virtq_desc *desc;
    struct virtq_avail *avail;
    struct virtq_used *used;
};

struct virtq_buf {
    void *addr;
    u32 len;
};

u32 virtio_device_id(u64 base);

/* Resets the device and negotiates VIRTIO_F_VERSION_1 plus the subset of `features` it offers.
 * Returns the negotiated subset, or -1. */
long virtio_begin(u64 base, u32 features);
int virtq_init(struct virtq *q, u64 base, u16 index, u16 size);
void virtio_ready(u64 base);
void virtio_reset(u64 base);
u8 virtio_config8(u64 base, u32 offset);
u32 virtio_config32(u64 base, u32 offset);
bool virtio_config_changed(u64 base);
void virtio_ack_config_change(u64 base);
bool virtio_failed(u64 base);

/* Posts a chain of `out` device-readable then `in` device-writable buffers. Returns its head or -1. */
int virtq_add(struct virtq *q, const struct virtq_buf *bufs, int out, int in);
void virtq_notify(struct virtq *q);
/* Returns the head of the next completed chain and frees it, or -1 if none. */
int virtq_next_used(struct virtq *q, u32 *len);

/* ---------------------------------------------------------------------------------------- */
/* vblk.c: block devices, for the file systems */

/* Returned by vblk_read and vblk_write while a medium change is pending, see vblk_media_changed. */
#define VBLK_CHANGED (-2)

struct vblk;
void vblk_init(void);
int vblk_count(void);
struct vblk *vblk_get(int index);
struct vblk *vblk_open(u64 base);
u64 vblk_capacity(const struct vblk *blk); /* in 512-byte sectors */
/* Returns whether the medium changed since the last call, and acknowledges the change. */
bool vblk_media_changed(struct vblk *blk);
bool vblk_change_pending(const struct vblk *blk);
bool vblk_read_only(const struct vblk *blk);
int vblk_read(struct vblk *blk, u64 sector, void *dst, u32 len);
int vblk_write(struct vblk *blk, u64 sector, const void *src, u32 len);

/* ---------------------------------------------------------------------------------------- */
/* vcon.c: named virtio-console ports, e.g. oc2's device bus, also known as vport0p<index> */

/* For the kernel */
int vcon_init(void);
bool vcon_named(int port);

/* For the application */
/* Returns the port's index, waiting for the device's handshake up to the deadline, or -1. */
int vcon_find(const char *name, u64 deadline);
/* Like vcon_find, and allocates the port's receive buffers on first use. */
int vcon_open(const char *name, u64 deadline);
bool vcon_readable(int port);
/* Whether vcon_send would succeed, possibly after vcon_reclaim. */
bool vcon_writable(int port);
/* Waits for data up to the deadline, then returns what is available, up to max. */
long vcon_read(int port, void *dst, size_t max, u64 deadline);
/*
 * Queues data for the host without copying it. The data must stay valid until vcon_reclaim returns
 * the descriptor head this returns. Returns -1 if all descriptors are in flight.
 */
int vcon_send(int port, const void *data, size_t len);
/* Returns the head of a completed vcon_send, or -1 if none completed. */
int vcon_reclaim(int port);
void vcon_ports(void (*fn)(void *ctx, int port, const char *name), void *ctx);
const char *vcon_name(int port);
/* Whether a port has unread data it was not reported for, or room again after a failed send. */
bool vcon_signal_ready(void);
/* Returns such a port and marks it reported, or -1. */
int vcon_take_signal(void);

#endif
