// SPDX-License-Identifier: MIT

use super::bytes;
use crate::drivers::uart::{self, Tty};
use core::ffi::{c_char, c_int};

/// -1 if there is no such port.
#[unsafe(no_mangle)]
unsafe extern "C" fn serial_find(name: *const c_char) -> c_int {
    uart::find(unsafe { bytes(name) }).map_or(-1, |tty| tty.0 as c_int)
}

/// -1 for a rate the port cannot do.
#[unsafe(no_mangle)]
extern "C" fn serial_set_baud(index: c_int, baud: u32) -> c_int {
    if usize::try_from(index).is_ok_and(|i| uart::set_baud(Tty(i), baud)) { 0 } else { -1 }
}

#[unsafe(no_mangle)]
extern "C" fn serial_baud(index: c_int) -> u32 {
    usize::try_from(index).ok().map(Tty).and_then(uart::baud).unwrap_or(0)
}
