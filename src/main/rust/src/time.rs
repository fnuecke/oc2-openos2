// SPDX-License-Identifier: MIT

use crate::api::time::TIME_NEVER;
use crate::arch::{IRQ_S_TIMER, csr_set, irq_disable, irq_enable, time_now, wfi};
use crate::board::board;
use crate::drivers::{console, rtc};
use crate::power;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering::Relaxed};

pub(crate) const FNV_OFFSET: u64 = 0xCBF29CE484222325;
const FNV_PRIME: u64 = 0x100000001B3;
pub(crate) const GOLDEN_GAMMA: u64 = 0x9E3779B97F4A7C15; // 2^64 / the golden ratio, splitmix64's increment

const SBI_EXT_TIME: u64 = 0x54494D45;

static EVENTS: AtomicU32 = AtomicU32::new(0);
static INPUT_TIMING: AtomicU64 = AtomicU64::new(0);

pub fn init() {
    csr_set!("sie", 1u64 << IRQ_S_TIMER);
    sbi_set_timer(TIME_NEVER);
}

pub fn interrupt() {
    sbi_set_timer(TIME_NEVER); // clears the pending bit; wait() rechecks the deadline
}

pub fn add_event() {
    EVENTS.store(EVENTS.load(Relaxed).wrapping_add(1), Relaxed);
}

pub fn events() -> u32 {
    EVENTS.load(Relaxed)
}

pub fn fnv1a(hash: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(hash, |hash, &c| (hash ^ c as u64).wrapping_mul(FNV_PRIME))
}

pub fn entropy_add(value: u64) {
    INPUT_TIMING.store((INPUT_TIMING.load(Relaxed) ^ value).wrapping_mul(FNV_PRIME), Relaxed);
}

pub fn entropy() -> u64 {
    let mut value = time_now() ^ INPUT_TIMING.load(Relaxed);
    for nanos in (0..rtc::count()).filter_map(rtc::read_nanos) {
        value = (value ^ nanos).wrapping_mul(FNV_PRIME);
    }
    value
}

pub fn mix64(mut x: u64) -> u64 {
    x = x.wrapping_add(GOLDEN_GAMMA);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D049BB133111EB);
    x ^ (x >> 31)
}

pub fn wait(mut ready: impl FnMut() -> bool, deadline: u64) -> bool {
    let mut result = false;
    loop {
        irq_disable();
        let now = time_now();
        if ready() {
            result = true;
            break;
        }
        if now >= deadline {
            break;
        }
        sbi_set_timer(deadline);
        wfi();
        irq_enable();
    }
    irq_enable();
    result
}

pub fn app_wait(mut ready: impl FnMut() -> bool, deadline: u64) -> bool {
    power::check();
    let mut result = false;
    let stop = || {
        result = ready();
        result || console::interrupted() || power::take_has_request()
    };
    wait(stop, deadline.min(power::deadline()));
    power::check();
    result
}

pub fn frequency() -> u64 {
    board().timebase
}

pub fn seconds_to_ticks(seconds: f64) -> u64 {
    let ticks = seconds * frequency() as f64;
    if ticks.is_nan() || ticks <= 0.0 {
        0
    } else if ticks >= (1u64 << 62) as f64 {
        1 << 62
    } else {
        ticks as u64
    }
}

#[derive(Default)]
pub struct Date {
    pub year: i64,
    pub month: i64,
    pub day: i64,
    pub hour: i64,
    pub min: i64,
    pub sec: i64,
}

// Howard Hinnant's public domain date algorithm.
pub fn civil_from_time(t: i64) -> Date {
    let mut out = Date::default();
    let mut days = t / 86400;
    let mut rest = t % 86400;
    if rest < 0 {
        rest += 86400;
        days -= 1;
    }
    out.hour = rest / 3600;
    out.min = rest / 60 % 60;
    out.sec = rest % 60;

    let z = days + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    out.day = doy - (153 * mp + 2) / 5 + 1;
    out.month = if mp < 10 { mp + 3 } else { mp - 9 };
    out.year = yoe + era * 400 + (out.month <= 2) as i64;
    out
}

#[inline(never)]
fn sbi_set_timer(value: u64) {
    unsafe { core::arch::asm!("ecall", inout("a0") value => _, out("a1") _, in("a6") 0, in("a7") SBI_EXT_TIME) };
}
