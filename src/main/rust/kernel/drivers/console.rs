// SPDX-License-Identifier: MIT

use super::uart;
use crate::api::InterruptHandler;
use crate::arch::IrqCell;

static STATE: IrqCell<State> = IrqCell::new(State { handler: None, interrupts: 0, unreported: 0, interrupted: false });

pub fn write(s: &[u8]) {
    uart::write_console(s);
}

pub fn set_interrupt_handler(handler: InterruptHandler) {
    STATE.with(|state| state.handler = handler);
}

pub fn clear_interrupt() {
    STATE.with(|state| state.interrupted = false);
}

pub fn interrupted() -> bool {
    STATE.with(|state| state.interrupted)
}

pub fn interrupts() -> u32 {
    STATE.with(|state| state.interrupts)
}

pub fn interrupt() -> bool {
    STATE.with(|state| {
        if state.handler.is_none() {
            return false;
        }
        state.interrupts = state.interrupts.wrapping_add(1);
        state.unreported += 1;
        state.interrupted = true;
        true
    })
}

pub fn notify() {
    let (handler, presses) = STATE.with(|state| (state.handler, core::mem::take(&mut state.unreported)));
    if let Some(handler) = handler {
        for _ in 0..presses {
            unsafe { handler() };
        }
    }
}

struct State {
    handler: InterruptHandler,
    interrupts: u32,
    unreported: u32,
    interrupted: bool,
}
