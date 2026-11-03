// SPDX-License-Identifier: MIT

use super::plic::HandlerData;
use super::{Driver, plic};
use crate::arch::{KernelCell, Mmio};
use crate::board::board;
use crate::fdt::Node;
use crate::power::{self, Request};

pub const DRIVER: Driver = Driver { compatible: &[b"sedna,syscon"], probe };
pub const POWEROFF: Driver = Driver { compatible: &[b"syscon-poweroff"], probe: |node| probe_write(node, &POWER_OFF) };
pub const REBOOT: Driver = Driver { compatible: &[b"syscon-reboot"], probe: |node| probe_write(node, &REBOOT_WRITE) };
const CONTROL: u64 = 0x08;
const STATUS: u64 = 0x0C;
const CONTROL_ENABLE: u32 = 1 << 1;
const STATUS_POWEROFF: u32 = 1 << 0;
const STATUS_RESET: u32 = 1 << 1;
static POWER_OFF: KernelCell<Write> = KernelCell::new(None);
static REBOOT_WRITE: KernelCell<Write> = KernelCell::new(None);

pub fn power_off() {
    perform(&POWER_OFF);
}

pub fn reboot() {
    perform(&REBOOT_WRITE);
}

fn probe(node: &Node) {
    let Some((base, _)) = node.reg() else { return };
    let syscon = unsafe { Mmio::new(base) };
    syscon.write32(CONTROL, CONTROL_ENABLE);
    if let Some(irq) = node.irq() {
        plic::register(irq, interrupt, HandlerData(base as usize));
    }
}

fn interrupt(HandlerData(base): HandlerData) {
    let syscon = unsafe { Mmio::new(base as u64) };
    let status = syscon.read32(STATUS);
    syscon.write32(STATUS, status); // clears it, and the interrupt
    if status & STATUS_POWEROFF != 0 {
        power::request(Request::PowerOff);
    } else if status & STATUS_RESET != 0 {
        power::request(Request::Reboot);
    }
}

fn probe_write(node: &Node, write: &KernelCell<Write>) {
    let (Some(regmap), Some(value)) = (node.u32(b"regmap"), node.u32(b"value")) else { return };
    let offset = node.u32(b"offset").unwrap_or(0);
    if let Some((base, _)) = board().fdt.with_phandle(regmap).and_then(|syscon| syscon.reg()) {
        write.with(|w| *w = Some((unsafe { Mmio::new(base + offset as u64) }, value)));
    }
}

fn perform(write: &KernelCell<Write>) {
    if let Some((register, value)) = write.with(|w| *w) {
        register.write32(0, value);
    }
}

type Write = Option<(Mmio, u32)>;
