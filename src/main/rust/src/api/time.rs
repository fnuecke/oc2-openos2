// SPDX-License-Identifier: MIT

use crate::drivers::rtc;
use crate::{arch, time};
use core::ffi::c_int;

pub const TIME_NEVER: u64 = u64::MAX;

#[unsafe(no_mangle)]
extern "C" fn time_now() -> u64 {
    arch::time_now()
}

#[unsafe(no_mangle)]
extern "C" fn time_frequency() -> u64 {
    time::frequency()
}

#[unsafe(no_mangle)]
extern "C" fn seconds_to_ticks(seconds: f64) -> u64 {
    time::seconds_to_ticks(seconds)
}

#[unsafe(no_mangle)]
extern "C" fn event_count() -> u32 {
    time::events()
}

/// Sleeps until event_count() differs from `seen` and returns true. Returns false at the deadline, while
/// a Ctrl+C is pending, and once after a power request. Waiting for a file to become readable:
///
///     uint32_t seen = event_count();
///     while (!(fs_poll(fd) & FS_POLL_IN) && event_wait(seen, deadline)) seen = event_count();
#[unsafe(no_mangle)]
extern "C" fn event_wait(seen: u32, deadline: u64) -> bool {
    time::app_wait(|| time::events() != seen, deadline)
}

#[unsafe(no_mangle)]
extern "C" fn sleep_until(deadline: u64) {
    time::app_wait(|| false, deadline);
}

#[unsafe(no_mangle)]
extern "C" fn rtc_seconds_of(index: c_int, seconds: &mut u64) -> bool {
    let Some(s) = usize::try_from(index).ok().and_then(rtc::seconds_of) else { return false };
    *seconds = s;
    true
}
