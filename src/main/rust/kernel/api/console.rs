// SPDX-License-Identifier: MIT

use super::{InterruptHandler, buffer};
use crate::drivers::console;
use core::ffi::{CStr, c_char};

#[unsafe(no_mangle)]
unsafe extern "C" fn console_write(s: *const c_char, len: usize) {
    console::write(unsafe { buffer(s.cast(), len) });
}

#[unsafe(no_mangle)]
unsafe extern "C" fn console_puts(s: *const c_char) {
    console::write(unsafe { CStr::from_ptr(s) }.to_bytes());
}

#[unsafe(no_mangle)]
extern "C" fn console_set_interrupt_handler(handler: InterruptHandler) {
    console::set_interrupt_handler(handler);
}

#[unsafe(no_mangle)]
extern "C" fn console_clear_interrupt() {
    console::clear_interrupt();
}

#[unsafe(no_mangle)]
extern "C" fn console_interrupts() -> u32 {
    console::interrupts()
}
