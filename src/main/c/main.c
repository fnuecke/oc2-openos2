/* SPDX-License-Identifier: MIT */

#include <string.h>
#include "drivers/console.h"
#include "drivers/plic.h"
#include "drivers/serial.h"
#include "drivers/syscon.h"
#include "drivers/virtio/virtio.h"
#include "fs/vfs.h"
#include "internal.h"

#define PTE_VALID 0x01
#define MEGAPAGE (2ull << 20)
#define PAGE 4096ull
#define STACK_FILL 0xA5

extern u8 _image_start[], _image_end[], _stack_guard[], _stack_bottom[], _stack_top[], _bss_start[], _bss_end[];
extern void trap_entry(void);
extern u64 boot_page_table[512];

static u64 kernel_megapages[512] __attribute__((aligned(4096)));
static u64 kernel_pages[512] __attribute__((aligned(4096)));

static u64 pte(const u64 physical, const u64 flags) {
    return (physical >> 12) << 10 | flags;
}

static void kernel_paging(void) {
    const u64 gigapage = 0x80000000ull;
    const u64 kernel = virt_to_phys(_image_start) & ~(MEGAPAGE - 1);
    for (u64 i = 0; i < 512; i++) {
        kernel_megapages[i] = pte(gigapage + i * MEGAPAGE, PTE_LEAF);
        kernel_pages[i] = pte(kernel + i * PAGE, PTE_LEAF);
    }
    for (u64 page = virt_to_phys(_stack_guard); page < virt_to_phys(_stack_bottom); page += PAGE) {
        kernel_pages[(page - kernel) / PAGE] = 0;
    }
    kernel_megapages[(kernel - gigapage) / MEGAPAGE] = pte(virt_to_phys(kernel_pages), PTE_VALID);
    boot_page_table[510] = pte(virt_to_phys(kernel_megapages), PTE_VALID);
    /* The identity mapping of RAM was only needed to switch to virtual addresses. */
    boot_page_table[2] = 0;
    boot_page_table[3] = 0;
    asm volatile("sfence.vma" ::: "memory");
}

static void fill_stack(void) {
    u8 *sp;
    asm volatile("mv %0, sp" : "=r"(sp));
    memset(_stack_bottom, STACK_FILL, (size_t) (sp - 256 - _stack_bottom));
}

#define MAPPED_RAM_START 0x80000000ull /* see boot.c */
#define MAPPED_RAM_END 0x100000000ull

static void add_region(const u64 start, const u64 end) {
    const u64 excluded[3][2] = { /* physical, by address */
        {board.reserved_base, board.reserved_base + board.reserved_size},
        {virt_to_phys(_image_start), virt_to_phys(_image_end)},
        {virt_to_phys(board.dtb) & ~(PAGE - 1), (virt_to_phys(board.dtb) + board.dtb_size + PAGE - 1) & ~(PAGE - 1)},
    };
    u64 at = start;
    for (int i = 0; i < 3; i++) {
        if (excluded[i][1] <= at || excluded[i][0] >= end) {
            continue;
        }
        if (excluded[i][0] > at) {
            heap_add(phys_to_virt(at), excluded[i][0] - at);
        }
        at = excluded[i][1];
    }
    if (end > at) {
        heap_add(phys_to_virt(at), end - at);
    }
}

static void add_memory(void) {
    for (int i = 0; i < board.ram_region_count; i++) {
        const u64 start = board.ram_region_base[i];
        const u64 end = start + board.ram_region_size[i];
        if (start < MAPPED_RAM_START || end > MAPPED_RAM_END) {
            kprintf("openos2: ignoring memory at %lx, outside the mapped 2 GiB\n", start);
            continue;
        }
        add_region(start, end);
    }
}

void kmain(const u64 dtb) {
    memset(_bss_start, 0, (size_t) (_bss_end - _bss_start)); /* the stack is above _bss_end */
    csr_write(stvec, (u64) trap_entry);
    kernel_paging();
    fill_stack();
    if (fdt_scan(phys_to_virt(dtb)) != 0) {
        halt();
    }
    time_init();
    plic_init();
    console_init();
    csr_set(sstatus, SSTATUS_SIE);
    add_memory();
    const u64 boot_ticks = time_now();

    vblk_init();
    vcon_init();
    serial_init();
    syscon_init();
    fs_mount_all();

    kprintf("openos2: %lu KiB RAM, %lu KiB heap, kernel %lu KiB, up %lu ms\n",
        board.ram_total / 1024, heap_total() / 1024,
        (u64) (_image_end - _image_start) / 1024,
        boot_ticks / (board.timebase / 1000));

    app_main();
    power_off();
}

void stack_usage(size_t *peak, size_t *size) {
    const u8 *p = _stack_bottom;
    while (p < _stack_top && *p == STACK_FILL) {
        p++;
    }
    *peak = (size_t) (_stack_top - p);
    *size = (size_t) (_stack_top - _stack_bottom);
}
