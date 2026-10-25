/* SPDX-License-Identifier: MIT */
#ifndef INTERNAL_H
#define INTERNAL_H

#include "kernel.h"

/* ---------------------------------------------------------------------------------------- */
/* Board */

#define MAX_VIRTIO 16
#define MAX_RTC 4
#define MAX_RAM 16
#define MAX_SERIAL 4

/* RAM is mapped at its physical address plus this; devices are identity mapped. See boot.c. */
#define RAM_VIRTUAL_OFFSET 0xFFFFFFFF00000000ull
#define PTE_LEAF 0xCF /* valid, readable, writable, executable, accessed, dirty */

static inline void *phys_to_virt(const u64 address) {
    return (void *) (address + RAM_VIRTUAL_OFFSET);
}

static inline u64 virt_to_phys(const void *address) {
    return (u64) address - RAM_VIRTUAL_OFFSET;
}

struct board {
    void *dtb; /* virtual */
    u64 dtb_size;
    u64 ram_base; /* physical */
    u64 ram_size; /* of the first memory node, where the kernel is */
    u64 ram_total; /* of all memory nodes */
    u64 ram_region_base[MAX_RAM]; /* physical, one per memory node, oc2 has one per stick */
    u64 ram_region_size[MAX_RAM];
    int ram_region_count;
    u64 reserved_base; /* the resident firmware */
    u64 reserved_size;
    u64 timebase;
    u64 uart; /* the console: /chosen's stdout-path, or the first 16550 */
    u32 uart_irq;
    u64 serial[MAX_SERIAL]; /* the other 16550s */
    u32 serial_irq[MAX_SERIAL];
    u32 serial_clock[MAX_SERIAL];
    int serial_count;
    u64 plic;
    int plic_context; /* S-mode context of hart 0, -1 if unknown */
    u64 rtc[MAX_RTC];
    int rtc_count;
    u64 poweroff_addr;
    u32 poweroff_value;
    u64 reboot_addr;
    u32 reboot_value;
    u64 syscon;
    u32 syscon_irq;
    u64 virtio[MAX_VIRTIO];
    u32 virtio_irq[MAX_VIRTIO];
    int virtio_count;
};

extern struct board board;

/* ---------------------------------------------------------------------------------------- */
/* time.c */

void time_init(void);
/* Called from the interrupt handler for the timer interrupt. */
void time_interrupt(void);
/* Ends the current wait_until(), like Ctrl+C does. Called from interrupt handlers. */
void wait_wake(void);
/* Drops a pending wait_wake(), so the next wait_until() waits. */
void wait_clear_wake(void);
/* Mixes a value into the entropy, e.g. when a byte arrived. Called from interrupt handlers. */
void entropy_add(u64 value);
/* Not random: copies of one machine started in the same RTC instants get the same value. */
u64 entropy(void);
/* splitmix64's finalizer: spreads every input bit over the result, e.g. for entropy(). */
u64 mix64(u64 x);

/* ---------------------------------------------------------------------------------------- */
/* power.c */

/* Records a power request from the host, see power_requested(). Called from the interrupt handler. */
void power_request(int request);

/* ---------------------------------------------------------------------------------------- */
/* alloc.c */

void heap_add(void *start, size_t size);

/* ---------------------------------------------------------------------------------------- */
/* fdt.c */

int fdt_scan(void *dtb);

/* ---------------------------------------------------------------------------------------- */
/* MMIO and CSR helpers */

#define SSTATUS_SIE (1ul << 1)

static inline u8 mmio_read8(u64 address) {
    return *(volatile u8 *) address;
}

static inline void mmio_write8(u64 address, u8 value) {
    *(volatile u8 *) address = value;
}

static inline u32 mmio_read32(u64 address) {
    return *(volatile u32 *) address;
}

static inline void mmio_write32(u64 address, u32 value) {
    *(volatile u32 *) address = value;
}

#define csr_read(csr) ({ u64 v; asm volatile("csrr %0, " #csr : "=r"(v)); v; })
#define csr_write(csr, value) asm volatile("csrw " #csr ", %0" :: "r"(value))
#define csr_set(csr, mask) asm volatile("csrs " #csr ", %0" :: "r"(mask))
#define csr_clear(csr, mask) asm volatile("csrc " #csr ", %0" :: "r"(mask))

#endif
