// SPDX-License-Identifier: MIT

use super::{DRIVERS, Driver};
use crate::arch::{KernelCell, Mmio};
use crate::fdt::Node;
use crate::fs::Name;
use crate::fs::sysfs::{self, CLASSES, Content};
use crate::time::civil_from_time;

register!(DRIVERS, &Driver { compatible: &[b"google,goldfish-rtc"], probe });
register!(CLASSES, &Clocks);

struct Clocks;

impl sysfs::Class for Clocks {
    fn name(&self) -> &'static [u8] {
        b"rtc"
    }

    fn count(&self) -> usize {
        count()
    }

    fn instance(&self, index: usize) -> Option<Name> {
        let mut name = Name::EMPTY;
        write!(name, "rtc{index}");
        Some(name)
    }

    fn attributes(&self) -> &'static [&'static [u8]] {
        &ATTRIBUTES
    }

    fn show(&self, index: usize, attribute: usize, out: &mut Content) -> Option<()> {
        let seconds = seconds_of(index)?;
        let d = civil_from_time(seconds as i64);
        match attribute {
            DATE => write!(out, "{:04}-{:02}-{:02}", d.year, d.month, d.day),
            SINCE_EPOCH => write!(out, "{seconds}"),
            TIME => write!(out, "{:02}:{:02}:{:02}", d.hour, d.min, d.sec),
            _ => return None,
        }
        Some(())
    }
}

const DATE: usize = 0;
const SINCE_EPOCH: usize = 1;
const TIME: usize = 2;
const ATTRIBUTES: [&[u8]; 3] = [b"date", b"since_epoch", b"time"];
const MAX_RTC: usize = 4;
const NANOS_PER_SECOND: u64 = 1_000_000_000;

static CLOCKS: KernelCell<[Option<Mmio>; MAX_RTC]> = KernelCell::new([None; MAX_RTC]);

fn probe(node: &Node) {
    let Some((base, _)) = node.reg() else { return };
    CLOCKS.with(|clocks| {
        if let Some(free) = clocks.iter_mut().find(|c| c.is_none()) {
            *free = Some(unsafe { Mmio::new(base) });
        }
    });
}

pub fn count() -> usize {
    CLOCKS.with(|clocks| clocks.iter().take_while(|c| c.is_some()).count())
}

pub fn read_nanos(index: usize) -> Option<u64> {
    let rtc = CLOCKS.with(|clocks| clocks.get(index).copied().flatten())?;
    let low = rtc.read32(0) as u64;
    let high = rtc.read32(4) as u64;
    Some(high << 32 | low)
}

pub fn seconds_of(index: usize) -> Option<u64> {
    read_nanos(index).map(|nanos| nanos / NANOS_PER_SECOND)
}

pub fn seconds() -> u64 {
    seconds_of(0).unwrap_or(0)
}
