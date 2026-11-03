// SPDX-License-Identifier: MIT
//! Turns on paging. We run at the physical load address before that.
//!
//! The kernel is linked at 0xFFFFFFFF80010000 so that the toolchain's medlow-model libc.a and
//! libgcc.a, which use absolute 32-bit sign-extended addresses, can be linked as they are. Sv39
//! with 1 GiB pages maps the top 2 GiB of the address space to physical 0x80000000. Devices (below
//! 2 GiB) are identity mapped, as is RAM. kmain() removes that mapping.

use crate::arch::RawCell;
use core::arch::asm;

const SATP_SV39: u64 = 8 << 60;
const GIB: u64 = 1 << 30;
pub const PTE_LEAF: u64 = 0xCF; // valid, readable, writable, executable, accessed, dirty

#[repr(C, align(4096))]
pub struct PageTable(pub [u64; 512]);

pub static BOOT_PAGE_TABLE: RawCell<PageTable> = RawCell::new(PageTable([0; 512]));

#[inline(always)]
pub fn pte(physical: u64, flags: u64) -> u64 {
    (physical >> 12) << 10 | flags
}

#[unsafe(no_mangle)]
extern "C" fn boot_paging() {
    let table = unsafe { &mut (*BOOT_PAGE_TABLE.get()).0 };
    table[0] = pte(0, PTE_LEAF); // devices, then RAM for the switch itself
    table[1] = pte(GIB, PTE_LEAF);
    table[2] = pte(2 * GIB, PTE_LEAF);
    table[3] = pte(3 * GIB, PTE_LEAF);
    table[510] = pte(2 * GIB, PTE_LEAF); // 0xFFFFFFFF80000000
    table[511] = pte(3 * GIB, PTE_LEAF);
    // No need for sfence.vma, Sedna flushes its TLB when satp.MODE changes.
    unsafe { asm!("csrw satp, {}", in(reg) SATP_SV39 | (table.as_ptr() as u64 >> 12)) };
}
