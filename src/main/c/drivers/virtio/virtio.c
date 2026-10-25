/* SPDX-License-Identifier: MIT */
/* virtio-mmio v2 transport and split virtqueues, shared by all virtio drivers. */

#include <stdlib.h>
#include <string.h>
#include "internal.h"
#include "virtio.h"

#define REG_MAGIC 0x000
#define REG_VERSION 0x004
#define REG_DEVICE_ID 0x008
#define REG_DEVICE_FEATURES 0x010
#define REG_DEVICE_FEATURES_SEL 0x014
#define REG_DRIVER_FEATURES 0x020
#define REG_DRIVER_FEATURES_SEL 0x024
#define REG_QUEUE_SEL 0x030
#define REG_QUEUE_SIZE_MAX 0x034
#define REG_QUEUE_SIZE 0x038
#define REG_QUEUE_READY 0x044
#define REG_QUEUE_NOTIFY 0x050
#define REG_INTERRUPT_STATUS 0x060
#define REG_INTERRUPT_ACK 0x064
#define REG_STATUS 0x070
#define REG_QUEUE_DESC_LOW 0x080
#define REG_QUEUE_DESC_HIGH 0x084
#define REG_QUEUE_DRIVER_LOW 0x090
#define REG_QUEUE_DRIVER_HIGH 0x094
#define REG_QUEUE_DEVICE_LOW 0x0A0
#define REG_QUEUE_DEVICE_HIGH 0x0A4

#define MAGIC 0x74726976

#define STATUS_ACKNOWLEDGE 1
#define STATUS_DRIVER 2
#define STATUS_DRIVER_OK 4
#define STATUS_FEATURES_OK 8
#define STATUS_DEVICE_NEEDS_RESET 64

#define F_VERSION_1 (1u << 0) /* feature bit 32 */

#define INTERRUPT_USED_BUFFER 1
#define INTERRUPT_CONFIG_CHANGE 2

#define DESC_F_NEXT 1
#define DESC_F_WRITE 2

void virtio_interrupt(const u32 irq) {
    for (int i = 0; i < board.virtio_count; i++) {
        if (board.virtio_irq[i] == irq) {
            const u64 base = board.virtio[i];
            /* A block device's config change stays pending for vblk.c, see virtio_config_changed.
             * Other drivers ignore config changes. */
            const u32 status = mmio_read32(base + REG_INTERRUPT_STATUS);
            const int keep = virtio_device_id(base) == VIRTIO_ID_BLOCK;
            mmio_write32(base + REG_INTERRUPT_ACK, keep ? status & INTERRUPT_USED_BUFFER : status);
        }
    }
}

u32 virtio_device_id(const u64 base) {
    if (mmio_read32(base + REG_MAGIC) != MAGIC || mmio_read32(base + REG_VERSION) != 2) {
        return 0;
    }
    return mmio_read32(base + REG_DEVICE_ID);
}

long virtio_begin(const u64 base, const u32 features) {
    mmio_write32(base + REG_STATUS, 0);
    mmio_write32(base + REG_STATUS, STATUS_ACKNOWLEDGE);
    mmio_write32(base + REG_STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER);

    mmio_write32(base + REG_DEVICE_FEATURES_SEL, 1);
    if ((mmio_read32(base + REG_DEVICE_FEATURES) & F_VERSION_1) == 0) {
        virtio_reset(base);
        return -1;
    }
    mmio_write32(base + REG_DEVICE_FEATURES_SEL, 0);
    const u32 offered = mmio_read32(base + REG_DEVICE_FEATURES);

    mmio_write32(base + REG_DRIVER_FEATURES_SEL, 1);
    mmio_write32(base + REG_DRIVER_FEATURES, F_VERSION_1);
    mmio_write32(base + REG_DRIVER_FEATURES_SEL, 0);
    mmio_write32(base + REG_DRIVER_FEATURES, offered & features);

    mmio_write32(base + REG_STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK);
    if ((mmio_read32(base + REG_STATUS) & STATUS_FEATURES_OK) == 0) {
        virtio_reset(base);
        return -1;
    }
    return offered & features;
}

int virtq_init(struct virtq *q, const u64 base, const u16 index, u16 size) {
    mmio_write32(base + REG_QUEUE_SEL, index);
    const u32 max = mmio_read32(base + REG_QUEUE_SIZE_MAX);
    if (max == 0) {
        return -1;
    }
    if (size > max) {
        size = (u16) max;
    }

    const size_t desc_bytes = 16u * size;
    const size_t avail_bytes = 6 + 2u * size;
    const size_t used_offset = (desc_bytes + avail_bytes + 3) & ~3u;
    const size_t used_bytes = 6 + 8u * size;
    /* Lives as long as the device, so it is never freed and the alignment slack is not tracked. */
    u8 *memory = malloc(used_offset + used_bytes + 15);
    if (memory == NULL) {
        return -1;
    }
    memory = (u8 *) (((u64) memory + 15) & ~15ull);
    memset(memory, 0, used_offset + used_bytes);

    q->base = base;
    q->index = index;
    q->size = size;
    q->desc = (struct virtq_desc *) memory;
    q->avail = (struct virtq_avail *) (memory + desc_bytes);
    q->used = (struct virtq_used *) (memory + used_offset);
    q->last_used = 0;
    for (u16 i = 0; i < size; i++) {
        q->desc[i].next = (u16) (i + 1);
    }
    q->free_head = 0;
    q->free_count = size;

    const u64 desc = virt_to_phys(q->desc);
    const u64 avail = virt_to_phys(q->avail);
    const u64 used = virt_to_phys(q->used);
    mmio_write32(base + REG_QUEUE_SIZE, size);
    mmio_write32(base + REG_QUEUE_DESC_LOW, (u32) desc);
    mmio_write32(base + REG_QUEUE_DESC_HIGH, (u32) (desc >> 32));
    mmio_write32(base + REG_QUEUE_DRIVER_LOW, (u32) avail);
    mmio_write32(base + REG_QUEUE_DRIVER_HIGH, (u32) (avail >> 32));
    mmio_write32(base + REG_QUEUE_DEVICE_LOW, (u32) used);
    mmio_write32(base + REG_QUEUE_DEVICE_HIGH, (u32) (used >> 32));
    mmio_write32(base + REG_QUEUE_READY, 1);
    return 0;
}

void virtio_ready(const u64 base) {
    mmio_write32(base + REG_STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK);
}

void virtio_reset(const u64 base) {
    mmio_write32(base + REG_STATUS, 0);
}

u8 virtio_config8(const u64 base, const u32 offset) {
    return mmio_read8(base + VIRTIO_REG_CONFIG + offset);
}

u32 virtio_config32(const u64 base, const u32 offset) {
    return mmio_read32(base + VIRTIO_REG_CONFIG + offset);
}

bool virtio_config_changed(const u64 base) {
    return (mmio_read32(base + REG_INTERRUPT_STATUS) & INTERRUPT_CONFIG_CHANGE) != 0;
}

void virtio_ack_config_change(const u64 base) {
    mmio_write32(base + REG_INTERRUPT_ACK, INTERRUPT_CONFIG_CHANGE);
}

bool virtio_failed(const u64 base) {
    return (mmio_read32(base + REG_STATUS) & STATUS_DEVICE_NEEDS_RESET) != 0;
}

int virtq_add(struct virtq *q, const struct virtq_buf *bufs, const int out, const int in) {
    const int count = out + in;
    if (count == 0 || count > q->free_count) {
        return -1;
    }
    const u16 head = q->free_head;
    u16 last = head;
    u16 index = head;
    for (int i = 0; i < count; i++) {
        struct virtq_desc *desc = &q->desc[index];
        desc->addr = virt_to_phys(bufs[i].addr);
        desc->len = bufs[i].len;
        desc->flags = (u16) ((i < out ? 0 : DESC_F_WRITE) | (i + 1 < count ? DESC_F_NEXT : 0));
        last = index;
        index = desc->next;
    }
    q->free_head = q->desc[last].next;
    q->free_count = (u16) (q->free_count - count);

    q->avail->ring[q->avail->idx % q->size] = head;
    asm volatile("fence w, w" ::: "memory");
    q->avail->idx++;
    return head;
}

void virtq_notify(struct virtq *q) {
    asm volatile("fence w, o" ::: "memory");
    mmio_write32(q->base + REG_QUEUE_NOTIFY, q->index);
}

int virtq_next_used(struct virtq *q, u32 *len) {
    if (*(volatile u16 *) &q->used->idx == q->last_used) {
        return -1;
    }
    asm volatile("fence r, r" ::: "memory");
    const struct virtq_used_elem *elem = &q->used->ring[q->last_used % q->size];
    const u16 head = (u16) elem->id;
    if (len != NULL) {
        *len = elem->len;
    }
    q->last_used++;

    u16 index = head;
    u16 count = 1;
    while (q->desc[index].flags & DESC_F_NEXT) {
        index = q->desc[index].next;
        count++;
    }
    q->desc[index].next = q->free_head;
    q->free_head = head;
    q->free_count = (u16) (q->free_count + count);

    /* Configuration changes stay pending for the driver to see, see virtio_config_changed. */
    if (mmio_read32(q->base + REG_INTERRUPT_STATUS) & INTERRUPT_USED_BUFFER) {
        mmio_write32(q->base + REG_INTERRUPT_ACK, INTERRUPT_USED_BUFFER);
    }
    return head;
}
