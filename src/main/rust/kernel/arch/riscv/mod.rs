// SPDX-License-Identifier: MIT

mod boot;
mod entry;

pub use boot::{BOOT_PAGE_TABLE, PTE_LEAF, PageTable, pte};
pub use entry::trap_entry;

use core::arch::asm;

const SSTATUS_SIE: u64 = 1 << 1;
pub const IRQ_S_TIMER: u64 = 5;
pub const IRQ_S_EXTERNAL: u64 = 9;

macro_rules! csr_write {
    ($csr:literal, $value:expr) => {
        unsafe { core::arch::asm!(concat!("csrw ", $csr, ", {}"), in(reg) $value) }
    };
}
macro_rules! csr_set {
    ($csr:literal, $mask:expr) => {
        unsafe { core::arch::asm!(concat!("csrs ", $csr, ", {}"), in(reg) $mask) }
    };
}
macro_rules! csr_clear {
    ($csr:literal, $mask:expr) => {
        unsafe { core::arch::asm!(concat!("csrc ", $csr, ", {}"), in(reg) $mask) }
    };
}
pub(crate) use {csr_set, csr_write};

#[inline(always)]
pub fn time_now() -> u64 {
    let value: u64;
    unsafe { asm!("rdtime {}", out(reg) value, options(nomem, nostack)) };
    value
}

#[inline(always)]
pub fn wfi() {
    unsafe { asm!("wfi") };
}

#[inline(always)]
pub fn sfence_vma() {
    unsafe { asm!("sfence.vma") };
}

pub fn irq_disable() {
    csr_clear!("sstatus", SSTATUS_SIE);
}

pub fn irq_enable() {
    csr_set!("sstatus", SSTATUS_SIE);
}

pub fn irq_enabled() -> bool {
    let sstatus: u64;
    unsafe { asm!("csrr {}, sstatus", out(reg) sstatus) };
    sstatus & SSTATUS_SIE != 0
}

/// Masks interrupts for `f` and restores the previous state, so it also works in a handler.
#[inline(always)]
pub(super) fn without_irq<R>(f: impl FnOnce() -> R) -> R {
    let previous: u64;
    unsafe { asm!("csrrc {}, sstatus, {}", out(reg) previous, in(reg) SSTATUS_SIE) };
    let result = f();
    csr_set!("sstatus", previous & SSTATUS_SIE);
    result
}
