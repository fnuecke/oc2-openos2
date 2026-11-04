// SPDX-License-Identifier: MIT

use super::plic::HandlerData;
use super::{DRIVERS, Driver, console, plic};
use crate::api::time::TIME_NEVER;
use crate::arch::{IrqCell, Mmio, irq_enabled, time_now};
use crate::board::board;
use crate::fdt::Node;
use crate::fs::devfs::{self, DEVICES, Poll};
use crate::fs::sysfs::{self, CLASSES, Content};
use crate::fs::{Error, Name, Result};
use crate::mem::HeapBuf;
use crate::signal::{self, Event, SOURCES, Value};
use crate::time::{entropy_add, wait};

register!(DRIVERS, &DRIVER);
register!(DEVICES, &Ttys);
register!(CLASSES, &Ttys);
register!(SOURCES, &Ttys);

const DRIVER: Driver = Driver { compatible: &[b"ns16550a", b"ns16550"], probe };

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Tty(pub usize);

struct Ttys;

impl devfs::Device for Ttys {
    fn count(&self) -> usize {
        count()
    }

    fn name(&self, index: usize) -> Option<Name> {
        name(Tty(index))
    }

    fn read(&self, index: usize, _: u64, dst: &mut [u8]) -> Result<usize> {
        read(Tty(index), dst)
    }

    fn write(&self, index: usize, _: u64, src: &[u8]) -> Result<usize> {
        with_port(index, |port| port.write(src)).unwrap_or(Err(Error::NotFound))
    }

    fn poll(&self, index: usize) -> Poll {
        let poll = with_port(index, |port| Poll { readable: port.readable(), writable: port.writable() });
        poll.unwrap_or(Poll { readable: false, writable: false })
    }

    fn nonblocking(&self) -> bool {
        true
    }
}

impl sysfs::Class for Ttys {
    fn name(&self) -> &'static [u8] {
        b"tty"
    }

    fn count(&self) -> usize {
        count()
    }

    fn instance(&self, index: usize) -> Option<Name> {
        name(Tty(index))
    }

    fn attributes(&self) -> &'static [&'static [u8]] {
        &[b"iomem_base"]
    }

    fn show(&self, index: usize, _: usize, out: &mut Content) -> Option<()> {
        write!(out, "{:#X}", base(Tty(index))?);
        Some(())
    }
}

impl signal::Source for Ttys {
    fn pending(&self) -> bool {
        PORTS.with(|ports| ports.iter().flatten().any(Port::pending))
    }

    fn take(&self) -> Option<Event> {
        let tty = PORTS.with(|ports| {
            let index = ports.iter().position(|port| port.as_ref().is_some_and(Port::pending))?;
            ports[index].as_mut()?.notified = true;
            Some(Tty(index))
        })?;
        let mut path = Value::of(b"/dev/");
        path.push(name(tty)?.as_bytes());
        Some(Event { value: Some(path), ..Event::new(c"serial") })
    }
}

const RBR: u64 = 0;
const THR: u64 = 0;
const DLL: u64 = 0;
const IER: u64 = 1;
const DLM: u64 = 1;
const FCR: u64 = 2;
const LCR: u64 = 3;
const LSR: u64 = 5;
const LSR_DR: u8 = 0x01;
const LSR_THRE: u8 = 0x20;
const LCR_8N1: u8 = 0x03;
const LCR_DLAB: u8 = 0x80;
const IER_RX: u8 = 0x01;
const IER_TX: u8 = 0x02;
const FCR_ENABLE_CLEAR: u8 = 0x07;

const CONSOLE: Tty = Tty(0);
const CTRL_C: u8 = 0x03;
const MAX_PORTS: usize = 5; // the console and 4 serial ports
const FIFO_SIZE: usize = 16;
const RX_SIZE: u32 = 1024;
const DEFAULT_BAUD: u32 = 9600;
static PORTS: IrqCell<[Option<Port>; MAX_PORTS]> = IrqCell::new([const { None }; MAX_PORTS]);

pub fn count() -> usize {
    MAX_PORTS
}

pub fn name(tty: Tty) -> Option<Name> {
    if !PORTS.with(|ports| ports.get(tty.0).is_some_and(Option::is_some)) {
        return None;
    }
    let mut name = Name::EMPTY;
    write!(name, "ttyS{}", tty.0);
    Some(name)
}

pub fn find(name: &[u8]) -> Option<Tty> {
    (0..count()).map(Tty).find(|&tty| self::name(tty).is_some_and(|n| n.as_bytes() == name))
}

pub fn base(tty: Tty) -> Option<u64> {
    with_port(tty.0, |port| port.uart.base())
}

pub fn read(tty: Tty, dst: &mut [u8]) -> Result<usize> {
    with_port(tty.0, |port| port.read(dst)).unwrap_or(Err(Error::NotFound))
}

pub fn readable(tty: Tty) -> bool {
    with_port(tty.0, Port::readable).unwrap_or(false)
}

pub fn console() -> Tty {
    PORTS
        .with(|ports| ports.iter().position(|port| port.as_ref().is_some_and(|port| port.console)))
        .map_or(CONSOLE, Tty)
}

pub fn set_console(tty: Tty) -> bool {
    PORTS.with(|ports| {
        if !ports.get(tty.0).is_some_and(Option::is_some) {
            return false;
        }
        for (index, port) in ports.iter_mut().enumerate() {
            if let Some(port) = port {
                port.console = false;
                if index == tty.0 {
                    port.drain();
                    port.rx.clear();
                    port.notified = false;
                    port.set_ier(port.ier | IER_RX);
                    port.console = true;
                }
            }
        }
        true
    })
}

pub fn write_console(s: &[u8]) {
    let Some((index, uart, interrupt)) = PORTS.with(|ports| {
        ports
            .iter()
            .enumerate()
            .find_map(|(i, port)| port.as_ref().filter(|p| p.console).map(|p| (i, p.uart, p.interrupt)))
    }) else {
        return;
    };
    let mut bytes = s.iter().flat_map(|&c| [(c == b'\n').then_some(b'\r'), Some(c)]).flatten().peekable();
    while bytes.peek().is_some() {
        let empty = || uart.read8(LSR) & LSR_THRE != 0;
        if !empty() {
            if interrupt && irq_enabled() {
                // The interrupt clears IER_TX again once the FIFO is empty.
                with_port(index, |port| port.set_ier(port.ier | IER_TX));
                wait(empty, TIME_NEVER);
            } else {
                while !empty() {}
            }
        }
        for c in bytes.by_ref().take(FIFO_SIZE) {
            uart.write8(THR, c);
        }
    }
}

pub fn set_baud(tty: Tty, baud: u32) -> bool {
    PORTS.with(|ports| ports.get_mut(tty.0).and_then(Option::as_mut).is_some_and(|port| port.set_baud(baud)))
}

pub fn baud(tty: Tty) -> Option<u32> {
    PORTS.with(|ports| ports.get(tty.0)?.as_ref().map(|port| port.baud))
}

fn probe(node: &Node) {
    let Some((base, _)) = node.reg() else { return };
    let stdout = board().stdout.filter(|&stdout| is_uart(stdout));
    let console = stdout.map_or(!PORTS.with(|ports| ports[CONSOLE.0].is_some()), |stdout| stdout == base);
    let Some(data) = HeapBuf::new(RX_SIZE as usize) else { return };
    let rx = Ring { data, head: 0, tail: 0 };

    let uart = unsafe { Mmio::new(base) };
    let clock = node.u32(b"clock-frequency");
    let index = PORTS.with(|ports| {
        let index = if console { CONSOLE.0 } else { (CONSOLE.0 + 1..MAX_PORTS).find(|&i| ports[i].is_none())? };
        let port =
            ports[index].insert(Port { uart, clock, baud: 0, rx, console, notified: false, ier: 0, interrupt: false });
        port.set_ier(0);

        if console {
            uart.write8(LCR, LCR_8N1);
        } else {
            port.set_baud(DEFAULT_BAUD);
        }

        port.drain();
        uart.write8(FCR, FCR_ENABLE_CLEAR);

        Some(index)
    });
    let Some(index) = index else { return };

    let registered = node.irq().is_some_and(|irq| plic::register(irq, interrupt, HandlerData(index)));
    with_port(index, |port| {
        port.interrupt = registered;
        port.set_ier(IER_RX);
    });
}

fn is_uart(base: u64) -> bool {
    let mut found = false;
    board()
        .fdt
        .walk(&mut |node| found |= node.is_compatible(DRIVER.compatible) && node.reg().is_some_and(|(r, _)| r == base));
    found
}

fn with_port<R>(index: usize, f: impl FnOnce(&mut Port) -> R) -> Option<R> {
    let result = PORTS.with(|ports| ports.get_mut(index)?.as_mut().map(f));
    console::notify();
    result
}

fn interrupt(HandlerData(index): HandlerData) {
    with_port(index, |port| {
        port.drain();
        if port.ier & IER_TX != 0 && port.uart.read8(LSR) & LSR_THRE != 0 {
            port.set_ier(port.ier & !IER_TX);
        }
    });
}

struct Port {
    uart: Mmio,
    clock: Option<u32>,
    baud: u32,
    rx: Ring,
    console: bool,
    notified: bool,
    ier: u8,
    interrupt: bool,
}

impl Port {
    fn set_ier(&mut self, ier: u8) {
        self.ier = ier;
        self.uart.write8(IER, ier);
    }

    fn drain(&mut self) {
        while self.uart.read8(LSR) & LSR_DR != 0 {
            if !self.rx.can_push() {
                self.set_ier(self.ier & !IER_RX);
                return;
            }
            let c = self.uart.read8(RBR);
            entropy_add(time_now() ^ c as u64);
            if self.console && c == CTRL_C && console::interrupt() {
                self.rx.clear();
                self.notified = false;
            } else {
                self.rx.push(c);
            }
        }
    }

    fn read(&mut self, dst: &mut [u8]) -> Result<usize> {
        self.drain();
        let mut count = 0;
        while count < dst.len()
            && let Some(c) = self.rx.pop()
        {
            dst[count] = c;
            count += 1;
        }
        self.set_ier(self.ier | IER_RX);
        self.notified = false;
        if count == 0 && !dst.is_empty() { Err(Error::Again) } else { Ok(count) }
    }

    fn write(&mut self, src: &[u8]) -> Result<usize> {
        if src.is_empty() {
            return Ok(0);
        }
        if !self.writable() {
            return Err(Error::Again);
        }
        let count = src.len().min(FIFO_SIZE);
        for &c in &src[..count] {
            self.uart.write8(THR, c);
        }
        Ok(count)
    }

    fn set_baud(&mut self, baud: u32) -> bool {
        let Some(clock) = self.clock.filter(|&clock| baud != 0 && baud <= clock / 16) else { return false };
        let divisor = (clock / (16 * baud)).min(0xFFFF);
        self.uart.write8(LCR, LCR_8N1 | LCR_DLAB);
        self.uart.write8(DLL, divisor as u8);
        self.uart.write8(DLM, (divisor >> 8) as u8);
        self.uart.write8(LCR, LCR_8N1);
        self.baud = baud;
        true
    }

    fn readable(&mut self) -> bool {
        self.drain();
        !self.rx.is_empty()
    }

    fn writable(&mut self) -> bool {
        if self.uart.read8(LSR) & LSR_THRE != 0 {
            return true;
        }
        self.set_ier(self.ier | IER_TX);
        false
    }

    fn pending(&self) -> bool {
        !self.notified && !self.rx.is_empty()
    }
}

struct Ring {
    data: HeapBuf, // RX_SIZE bytes
    head: u32,
    tail: u32,
}

impl Ring {
    fn is_empty(&self) -> bool {
        self.head == self.tail
    }

    fn can_push(&self) -> bool {
        self.head.wrapping_sub(self.tail) < RX_SIZE
    }

    fn clear(&mut self) {
        self.tail = self.head;
    }

    fn push(&mut self, c: u8) {
        self.data[(self.head % RX_SIZE) as usize] = c;
        self.head = self.head.wrapping_add(1);
    }

    fn pop(&mut self) -> Option<u8> {
        if self.is_empty() {
            return None;
        }
        let c = self.data[(self.tail % RX_SIZE) as usize];
        self.tail = self.tail.wrapping_add(1);
        Some(c)
    }
}
