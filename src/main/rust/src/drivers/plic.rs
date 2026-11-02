// SPDX-License-Identifier: MIT

use crate::arch::{IRQ_S_EXTERNAL, IrqCell, Mmio, csr_set};
use crate::board::board;

pub type Handler = fn(HandlerData);

#[derive(Clone, Copy)]
pub struct HandlerData(pub usize);

const PRIORITY: u64 = 0x0;
const ENABLE: u64 = 0x2000;
const ENABLE_STRIDE: u64 = 0x80;
const THRESHOLD: u64 = 0x200000;
const CLAIM: u64 = 0x200004;
const CONTEXT_STRIDE: u64 = 0x1000;
const MAX_HANDLERS: usize = 24;
static LINES: IrqCell<[Option<Line>; MAX_HANDLERS]> = IrqCell::new([None; MAX_HANDLERS]);

pub fn init() {
    plic().write32(context() * CONTEXT_STRIDE + THRESHOLD, 0);
    csr_set!("sie", 1u64 << IRQ_S_EXTERNAL);
}

pub fn register(irq: u32, handler: Handler, data: HandlerData) -> bool {
    let registered = LINES.with(|lines| {
        let Some(free) = lines.iter_mut().find(|line| line.is_none()) else { return false };
        *free = Some(Line { irq, handler, data });
        true
    });
    if registered {
        plic().write32(PRIORITY + 4 * irq as u64, 1);
        set_enabled(irq, true);
    }
    registered
}

pub fn set_enabled(irq: u32, enabled: bool) {
    let enable = ENABLE + context() * ENABLE_STRIDE + 4 * (irq / 32) as u64;
    let bits = plic().read32(enable);
    let bit = 1 << (irq % 32);
    plic().write32(enable, if enabled { bits | bit } else { bits & !bit });
}

pub fn dispatch() {
    let claim = context() * CONTEXT_STRIDE + CLAIM;
    let irq = plic().read32(claim);
    if irq == 0 {
        return;
    }
    LINES.with(|lines| {
        for line in lines.iter().flatten().filter(|line| line.irq == irq) {
            (line.handler)(line.data);
        }
    });
    plic().write32(claim, irq);
}

fn plic() -> Mmio {
    unsafe { Mmio::new(board().plic) }
}

fn context() -> u64 {
    board().plic_context
}

#[derive(Clone, Copy)]
struct Line {
    irq: u32,
    handler: Handler,
    data: HandlerData,
}
