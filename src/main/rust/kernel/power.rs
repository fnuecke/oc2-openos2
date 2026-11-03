// SPDX-License-Identifier: MIT

use crate::api::time::TIME_NEVER;
use crate::api::{InterruptHandler, kprintf};
use crate::arch::{IrqCell, csr_write, irq_disable, time_now, wfi};
use crate::board::board;
use crate::drivers::syscon;
use crate::fs::vfs::unmount_all;
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering::Relaxed};

#[derive(Clone, Copy, PartialEq)]
pub enum Request {
    PowerOff = 1,
    Reboot = 2,
}

const REQUEST_TIMEOUT_S: u64 = 5;

static REQUESTED: AtomicU8 = AtomicU8::new(0); // a Request, 0 for none
static REQUESTED_AT: AtomicU64 = AtomicU64::new(0);
static UNSEEN: AtomicBool = AtomicBool::new(false);
static REQUEST_HANDLER: IrqCell<InterruptHandler> = IrqCell::new(None);

pub fn request(request: Request) {
    if REQUESTED.load(Relaxed) != 0 {
        return;
    }
    REQUESTED.store(request as u8, Relaxed);
    REQUESTED_AT.store(time_now(), Relaxed);
    UNSEEN.store(true, Relaxed);
    if let Some(handler) = REQUEST_HANDLER.with(|h| *h) {
        unsafe { handler() };
    }
}

pub fn requested() -> Option<Request> {
    match REQUESTED.load(Relaxed) {
        r if r == Request::PowerOff as u8 => Some(Request::PowerOff),
        r if r == Request::Reboot as u8 => Some(Request::Reboot),
        _ => None,
    }
}

pub fn take_has_request() -> bool {
    UNSEEN.swap(false, Relaxed)
}

pub fn set_handler(handler: InterruptHandler) {
    REQUEST_HANDLER.with(|h| *h = handler);
}

pub fn halt() -> ! {
    irq_disable();
    csr_write!("sie", 0u64);
    loop {
        wfi();
    }
}

pub fn power_off() -> ! {
    unmount_all();
    syscon::power_off();
    unsafe { kprintf(c"power off failed, halting\n".as_ptr()) };
    halt()
}

pub fn reboot() -> ! {
    unmount_all();
    syscon::reboot();
    unsafe { kprintf(c"reboot failed, halting\n".as_ptr()) };
    halt()
}

pub fn deadline() -> u64 {
    if requested().is_some() { REQUESTED_AT.load(Relaxed) + REQUEST_TIMEOUT_S * board().timebase } else { TIME_NEVER }
}

pub fn check() {
    let Some(request) = requested() else { return };
    if time_now() < deadline() {
        return;
    }
    let (what, act): (_, fn() -> !) = match request {
        Request::PowerOff => (c"power off", power_off),
        Request::Reboot => (c"reboot", reboot),
    };
    unsafe { kprintf(c"\n%s requested and not answered, unmounting\n".as_ptr(), what.as_ptr()) };
    act()
}
