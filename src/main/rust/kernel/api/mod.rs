// SPDX-License-Identifier: MIT

pub mod console;
pub mod error;
pub mod fs;
pub mod mem;
pub mod power;
pub mod serial;
pub mod signal;
pub mod time;

pub use error::{Error, Result};

use core::ffi::{CStr, c_char, c_int, c_long};

pub type InterruptHandler = Option<unsafe extern "C" fn()>;

unsafe extern "C" {
    pub fn app_main();

    pub fn kprintf(format: *const c_char, ...);
}

pub unsafe fn buffer<'a>(p: *const u8, len: usize) -> &'a [u8] {
    if len == 0 { &[] } else { unsafe { core::slice::from_raw_parts(p, len) } }
}

pub unsafe fn buffer_mut<'a>(p: *mut u8, len: usize) -> &'a mut [u8] {
    if len == 0 { &mut [] } else { unsafe { core::slice::from_raw_parts_mut(p, len) } }
}

pub unsafe fn bytes<'a>(s: *const c_char) -> &'a [u8] {
    unsafe { CStr::from_ptr(s) }.to_bytes()
}

fn code(result: Result<()>) -> c_int {
    result.err().map_or(0, |e| e as c_int)
}

fn int(result: Result<usize>) -> c_int {
    result.map_or_else(|e| e as c_int, |n| n as c_int)
}

fn long(result: Result<usize>) -> c_long {
    result.map_or_else(|e| e as c_long, |n| n as c_long)
}
