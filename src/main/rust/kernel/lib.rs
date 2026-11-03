// SPDX-License-Identifier: MIT

#![no_std]

mod api;
mod arch;
mod board;
mod bytes;
mod drivers;
mod fdt;
mod fs;
mod kmain;
mod libc;
mod mem;
mod power;
mod signal;
mod text;
mod time;
mod trap;

// Custom one to drop some strings to save a few KB.
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe { api::kprintf(c"\n*** kernel panic\n*** system halted\n".as_ptr()) };
    power::halt()
}
