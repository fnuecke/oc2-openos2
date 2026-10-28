/* SPDX-License-Identifier: MIT */
/* Polled virtio-blk: one request in flight per device. */

#include <stdlib.h>
#include "internal.h"
#include "virtio.h"

#define REQUEST_IN 0
#define REQUEST_OUT 1

#define QUEUE_SIZE 4

#define F_RO (1u << 5)

struct request {
    u32 type;
    u32 reserved;
    u64 sector;
};

struct vblk {
    struct virtq queue;
    u64 capacity;
    int read_only;
    struct request request;
    volatile u8 status;
};

static struct vblk *devices[MAX_VIRTIO];
static int device_count;

void vblk_init(void) {
    for (int i = 0; i < board.virtio_count; i++) {
        struct vblk *blk = vblk_open(board.virtio[i]);
        if (blk != NULL) {
            devices[device_count++] = blk;
        }
    }
}

int vblk_count(void) {
    return device_count;
}

struct vblk *vblk_get(const int index) {
    return devices[index];
}

struct vblk *vblk_open(const u64 base) {
    if (virtio_device_id(base) != VIRTIO_ID_BLOCK) {
        return NULL;
    }
    const long features = virtio_begin(base, F_RO);
    if (features < 0) {
        return NULL;
    }
    struct vblk *blk = calloc(1, sizeof(*blk));
    if (blk == NULL || virtq_init(&blk->queue, base, 0, QUEUE_SIZE) != 0) {
        virtio_reset(base);
        free(blk);
        return NULL;
    }
    virtio_ready(base);
    blk->read_only = (features & F_RO) != 0;
    blk->capacity = virtio_config32(base, 0) | ((u64) virtio_config32(base, 4) << 32);
    return blk;
}

u64 vblk_capacity(const struct vblk *blk) {
    return blk->capacity;
}

bool vblk_media_changed(struct vblk *blk) {
    const u64 base = blk->queue.base;
    if (!virtio_config_changed(base)) {
        return 0;
    }
    virtio_ack_config_change(base);
    blk->capacity = virtio_config32(base, 0) | ((u64) virtio_config32(base, 4) << 32);
    return 1;
}

bool vblk_change_pending(const struct vblk *blk) {
    return virtio_config_changed(blk->queue.base);
}

bool vblk_read_only(const struct vblk *blk) {
    return blk->read_only;
}

static int transfer(struct vblk *blk, const u32 type, const u64 sector, void *data, const u32 len) {
    if (virtio_config_changed(blk->queue.base)) {
        return VBLK_CHANGED;
    }
    if (len == 0 || len % VBLK_SECTOR_SIZE != 0 || sector + len / VBLK_SECTOR_SIZE > blk->capacity) {
        return -1;
    }
    blk->request.type = type;
    blk->request.reserved = 0;
    blk->request.sector = sector;
    blk->status = 0xFF;

    const struct virtq_buf bufs[3] = {
        {&blk->request, sizeof(blk->request)},
        {data, len},
        {(void *) &blk->status, 1},
    };
    const int out = type == REQUEST_OUT ? 2 : 1;
    if (virtq_add(&blk->queue, bufs, out, 3 - out) < 0) {
        return -1;
    }
    virtq_notify(&blk->queue);
    while (virtq_next_used(&blk->queue, NULL) < 0) {
        if (virtio_failed(blk->queue.base)) {
            return -1;
        }
    }
    if (blk->status != 0 && virtio_config_changed(blk->queue.base)) {
        return VBLK_CHANGED;
    }
    return blk->status == 0 ? 0 : -1;
}

int vblk_read(struct vblk *blk, const u64 sector, void *dst, const u32 len) {
    return transfer(blk, REQUEST_IN, sector, dst, len);
}

int vblk_write(struct vblk *blk, const u64 sector, const void *src, const u32 len) {
    return transfer(blk, REQUEST_OUT, sector, (void *) src, len);
}
