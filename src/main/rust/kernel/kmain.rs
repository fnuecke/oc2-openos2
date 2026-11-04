// SPDX-License-Identifier: MIT

use crate::api::{app_main, kprintf};
use crate::arch::{BOOT_PAGE_TABLE, PTE_LEAF, PageTable, RawCell, csr_write, irq_enable, pte, sfence_vma, trap_entry};
use crate::board::{self, board, phys_to_virt, virt_to_phys};
use crate::drivers::{self, plic};
use crate::fs::vfs;
use crate::libc::{malloc, stdio};
use crate::{mem, power, time};

pub struct Sym(*const u8);

unsafe impl Sync for Sym {}

impl Sym {
    pub fn addr(&self) -> u64 {
        self.0 as u64
    }
}

static IMAGE_START: Sym = Sym(&raw const _image_start);
static IMAGE_END: Sym = Sym(&raw const _image_end);
pub static STACK_GUARD: Sym = Sym(&raw const _stack_guard);
pub static STACK_BOTTOM: Sym = Sym(&raw const _stack_bottom);
static STACK_TOP: Sym = Sym(&raw const _stack_top);

const PTE_VALID: u64 = 0x01;
const MEGAPAGE: u64 = 2 << 20;
const PAGE: u64 = 4096;
const STACK_FILL: u8 = 0xA5;
const MAPPED_RAM_START: u64 = 0x8000_0000; // the gigapages of arch::riscv::boot
const MAPPED_RAM_END: u64 = 0x1_0000_0000;

unsafe extern "C" {
    static _image_start: u8;
    static _image_end: u8;
    static _stack_guard: u8;
    static _stack_bottom: u8;
    static _stack_top: u8;
    static _kernel_page_tables: RawCell<[PageTable; 2]>;
}

pub fn stack_usage() -> (usize, usize) {
    let (bottom, top) = (STACK_BOTTOM.addr(), STACK_TOP.addr());
    let mut p = bottom;
    while p < top && unsafe { *(p as *const u8) } == STACK_FILL {
        p += 1;
    }
    ((top - p) as usize, (top - bottom) as usize)
}

/// Called by the entry, see arch::riscv::entry.
#[unsafe(no_mangle)]
extern "C" fn kmain(dtb: u64) -> ! {
    csr_write!("stvec", trap_entry as *const () as usize);
    kernel_paging();
    fill_stack();
    if unsafe { board::scan(phys_to_virt(dtb) as *const u8) }.is_err() {
        power::halt();
    }
    time::init();
    plic::init();
    irq_enable();
    let unmapped = add_memory();
    drivers::probe();
    if let Some(start) = unmapped {
        unsafe { kprintf(c"system: ignoring memory at %lx\n".as_ptr(), start) };
    }
    vfs::mount_all();

    unsafe {
        kprintf(c"system: %lu KiB RAM, %lu KiB heap\n".as_ptr(), board().ram_total / 1024, mem::heap_total() / 1024);
        app_main();
    }
    stdio::flush_all();
    power::power_off()
}

fn kernel_paging() {
    let ([megapages, pages], root) = unsafe { (&mut *_kernel_page_tables.get(), &mut *BOOT_PAGE_TABLE.get()) };
    let (megapages, pages, root) = (&mut megapages.0, &mut pages.0, &mut root.0);
    let gigapage = MAPPED_RAM_START;
    let kernel = virt_to_phys(IMAGE_START.addr()) & !(MEGAPAGE - 1);
    for i in 0..512 {
        megapages[i] = pte(gigapage + i as u64 * MEGAPAGE, PTE_LEAF);
        pages[i] = pte(kernel + i as u64 * PAGE, PTE_LEAF);
    }
    let mut page = virt_to_phys(STACK_GUARD.addr());
    while page < virt_to_phys(STACK_BOTTOM.addr()) {
        pages[((page - kernel) / PAGE) as usize] = 0;
        page += PAGE;
    }
    megapages[((kernel - gigapage) / MEGAPAGE) as usize] = pte(virt_to_phys(pages.as_ptr() as u64), PTE_VALID);
    root[510] = pte(virt_to_phys(megapages.as_ptr() as u64), PTE_VALID);
    // The identity mapping of RAM was only needed to switch to virtual addresses.
    root[2] = 0;
    root[3] = 0;
    sfence_vma();
}

fn fill_stack() {
    let sp: u64;
    unsafe { core::arch::asm!("mv {}, sp", out(reg) sp) };
    let bottom = STACK_BOTTOM.addr();
    unsafe { core::ptr::write_bytes(bottom as *mut u8, STACK_FILL, (sp - 256 - bottom) as usize) };
}

fn add_region(start: u64, end: u64) {
    let b = board();
    let dtb = virt_to_phys(b.fdt.as_bytes().as_ptr() as u64);
    let excluded = [
        // physical, by address
        (b.reserved.base, b.reserved.base + b.reserved.size),
        (virt_to_phys(IMAGE_START.addr()), virt_to_phys(IMAGE_END.addr())),
        (dtb & !(PAGE - 1), (dtb + b.fdt.as_bytes().len() as u64 + PAGE - 1) & !(PAGE - 1)),
    ];
    let mut at = start;
    for (from, to) in excluded {
        if to <= at || from >= end {
            continue;
        }
        if from > at {
            unsafe { malloc::add(phys_to_virt(at), from - at) };
        }
        at = to;
    }
    if end > at {
        unsafe { malloc::add(phys_to_virt(at), end - at) };
    }
}

fn add_memory() -> Option<u64> {
    let mut unmapped = None;
    for region in &board().ram[..board().ram_count] {
        let (start, end) = (region.base, region.base + region.size);
        if start < MAPPED_RAM_START || end > MAPPED_RAM_END {
            unmapped = unmapped.or(Some(start));
            continue;
        }
        add_region(start, end);
    }
    unmapped
}
