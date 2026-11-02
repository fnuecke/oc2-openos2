// SPDX-License-Identifier: MIT
//! Called from the trap entry in assembly code, see arch::riscv::entry.

use crate::api::kprintf;
use crate::arch::{IRQ_S_EXTERNAL, IRQ_S_TIMER};
use crate::drivers::plic;
use crate::kmain::{STACK_BOTTOM, STACK_GUARD};
use crate::power::halt;
use crate::time;

const SCAUSE_INTERRUPT: u64 = 1 << 63;

#[unsafe(no_mangle)]
extern "C" fn handle_interrupt(cause: u64) {
    match cause & !SCAUSE_INTERRUPT {
        IRQ_S_TIMER => time::interrupt(),
        IRQ_S_EXTERNAL => plic::dispatch(),
        _ => {}
    }
    time::add_event();
}

#[unsafe(no_mangle)]
extern "C" fn trap_report(cause: u64, epc: u64, tval: u64, sp: u64, ra: u64) -> ! {
    if (STACK_GUARD.addr()..STACK_BOTTOM.addr()).contains(&tval) {
        unsafe { kprintf(c"\n*** kernel stack overflow".as_ptr()) };
    }
    unsafe {
        kprintf(
            c"\n*** kernel trap: scause=%#lx sepc=%#lx stval=%#lx sp=%#lx ra=%#lx\n".as_ptr(),
            cause,
            epc,
            tval,
            sp,
            ra,
        );
        kprintf(c"*** system halted\n".as_ptr());
    }
    halt()
}
