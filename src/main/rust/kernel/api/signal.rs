// SPDX-License-Identifier: MIT

use crate::arch::KernelCell;
use crate::signal::{self, Event};
use core::ffi::{CStr, c_char};

/// Only valid until the next signal_next().
#[repr(C)]
pub struct Signal {
    name: *const c_char,
    value: *const c_char,
    value_length: usize,
    detail: *const c_char, // NUL-terminated
    int_values: [i64; 2],
    int_value_count: usize,
}

impl Signal {
    const NONE: Self = Self {
        name: core::ptr::null(),
        value: core::ptr::null(),
        value_length: 0,
        detail: core::ptr::null(),
        int_values: [0; 2],
        int_value_count: 0,
    };

    fn of(event: &Event) -> Self {
        Self {
            name: event.name.as_ptr(),
            value: event.value.as_ref().map_or(core::ptr::null(), |value| value.as_ptr()),
            value_length: event.value.as_ref().map_or(0, |value| value.len()),
            detail: event.detail.map_or(core::ptr::null(), CStr::as_ptr),
            int_values: event.int_values,
            int_value_count: event.int_value_count,
        }
    }
}

/// The data for the values of the last signal returned from signal_next.
static LAST: KernelCell<Option<Event>> = KernelCell::new(None);

#[unsafe(no_mangle)]
extern "C" fn signal_wait(deadline: u64) {
    signal::wait(deadline);
}

#[unsafe(no_mangle)]
extern "C" fn signal_collect() {
    signal::collect();
}

#[unsafe(no_mangle)]
extern "C" fn signal_next(out: &mut Signal) -> bool {
    let next = signal::next();
    let found = next.is_some();
    *out = LAST.with(|last| {
        *last = next;
        last.as_ref().map_or(Signal::NONE, Signal::of)
    });
    found
}
