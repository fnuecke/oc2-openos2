/* SPDX-License-Identifier: MIT */
/*
 * Turns on paging. Runs at the physical load address before that, so this file is compiled with
 * the medany code model and without linker relaxation: it may only use PC-relative addressing and
 * must not call into other files.
 *
 * The kernel is linked at 0xFFFFFFFF80010000 so that the toolchain's medlow-model libc.a and
 * libgcc.a, which use absolute 32-bit sign-extended addresses, can be linked as they are. Sv39
 * with 1 GiB pages maps the top 2 GiB of the address space to physical 0x80000000. Devices (below
 * 2 GiB) are identity mapped, and so is RAM, up until we remove the mapping in main.c.
 */

#include "internal.h"

#define SATP_SV39 (8ull << 60)
#define GIB (1ull << 30)

u64 boot_page_table[512] __attribute__((aligned(4096), section(".data.page_table")));

static u64 gigapage(const u64 physical) {
    return (physical >> 12) << 10 | PTE_LEAF;
}

void boot_paging(void) {
    for (u64 i = 0; i < 4; i++) {
        boot_page_table[i] = gigapage(i * GIB); /* devices, then RAM for the switch itself */
    }
    boot_page_table[510] = gigapage(2 * GIB); /* 0xFFFFFFFF80000000 */
    boot_page_table[511] = gigapage(3 * GIB);
    /* No need for sfence.vma, Sedna flushes its TLB when satp.MODE changes. */
    csr_write(satp, SATP_SV39 | ((u64) boot_page_table >> 12));
}
