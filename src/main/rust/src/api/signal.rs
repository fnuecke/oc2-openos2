// SPDX-License-Identifier: MIT

use crate::arch::KernelCell;
use crate::drivers::virtio::console as vcon;
use crate::fs::vfs::Change;
use crate::power::Request;
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

    fn new(name: &'static CStr, value: &[u8]) -> Self {
        Self { name: name.as_ptr(), value: value.as_ptr().cast(), value_length: value.len(), ..Self::NONE }
    }

    fn of(event: &Event) -> Self {
        match event {
            Event::Medium(path, change) => {
                let detail = match change {
                    Change::Added => c"added",
                    Change::Removed => c"removed",
                    Change::Changed => c"changed",
                };
                Self { detail: detail.as_ptr(), ..Self::new(c"medium", path.as_bytes()) }
            }
            Event::Port(port) => {
                let name = vcon::name_ptr(*port).unwrap_or(core::ptr::null());
                let length = if name.is_null() { 0 } else { unsafe { CStr::from_ptr(name) }.count_bytes() };
                Self { name: c"port".as_ptr(), value: name, value_length: length, ..Self::NONE }
            }
            Event::Serial(path) => Self::new(c"serial", path.as_bytes()),
            Event::Sound(path) => Self::new(c"sound", path.as_bytes()),
            Event::Key(key) => Self {
                int_values: [key.ch.map_or(-1, i64::from), key.code as i64],
                int_value_count: 2,
                ..Self::new(if key.down { c"key_down" } else { c"key_up" }, key.keyboard.as_bytes())
            },
            Event::Power(Request::PowerOff) => Self { name: c"shutdown".as_ptr(), ..Self::NONE },
            Event::Power(Request::Reboot) => Self { name: c"reboot".as_ptr(), ..Self::NONE },
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
