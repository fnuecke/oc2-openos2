// SPDX-License-Identifier: MIT

use super::InterruptHandler;
use crate::libc::stdio;
use crate::power::{self, Request};
use core::ffi::c_int;

pub const POWER_OFF_REQUEST: c_int = 1;
pub const POWER_REBOOT_REQUEST: c_int = 2;

#[unsafe(no_mangle)]
extern "C" fn power_off() -> ! {
    stdio::flush_all();
    power::power_off()
}

#[unsafe(no_mangle)]
extern "C" fn power_reboot() -> ! {
    stdio::flush_all();
    power::reboot()
}

#[unsafe(no_mangle)]
extern "C" fn power_requested() -> c_int {
    match power::requested() {
        Some(Request::PowerOff) => POWER_OFF_REQUEST,
        Some(Request::Reboot) => POWER_REBOOT_REQUEST,
        None => 0,
    }
}

/// Called when the host asks to power off or reboot. The application has 5 seconds to call
/// power_off() or power_reboot(), otherwise the kernel does on the next opportunity.
#[unsafe(no_mangle)]
extern "C" fn power_set_interrupt_handler(handler: InterruptHandler) {
    power::set_handler(handler);
}

/// Must not be called while a file system call is in flight.
#[unsafe(no_mangle)]
extern "C" fn power_check() {
    power::check();
}
