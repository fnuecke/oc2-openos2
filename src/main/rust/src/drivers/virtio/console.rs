// SPDX-License-Identifier: MIT

use super::{Buf, Device, Driver, Virtq};
use crate::arch::KernelCell;
use crate::bytes::until_nul;
use crate::fs::devfs::{self, Poll};
use crate::fs::sysfs::{self, Content};
use crate::fs::{Error, Name, OpenFlags, Result};
use crate::mem::{HeapBuf, free, malloc};
use core::ffi::c_char;

pub const DRIVER: Driver = Driver { id: 3, probe, keeps_config_change: false, interrupt_hook: None };

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Vport(pub usize);

pub struct Ports;

impl devfs::Device for Ports {
    fn count(&self) -> usize {
        port_count()
    }

    fn name(&self, index: usize) -> Option<Name> {
        vport_name(Vport(index))
    }

    fn find(&self, name: &[u8]) -> Option<usize> {
        STATE.with(|state| state.find(name))
    }

    fn open(&self, index: usize, _: OpenFlags) -> Result<usize> {
        STATE.with(|state| {
            if state.ports[index].opened {
                return Ok(index);
            }
            state.ports[index].open().map_err(|_| Error::NoSpace)?;
            state.send_control(index as u32, PORT_OPEN, 1);
            Ok(index)
        })
    }

    fn read(&self, index: usize, _: u64, dst: &mut [u8]) -> Result<usize> {
        STATE.with(|state| {
            state.pump_control();
            let port = &mut state.ports[index];
            port.notified = false;
            match port.read(dst) {
                0 if !dst.is_empty() => Err(Error::Again),
                count => Ok(count),
            }
        })
    }

    fn write(&self, index: usize, _: u64, src: &[u8]) -> Result<usize> {
        if src.is_empty() {
            return Ok(0);
        }
        STATE.with(|state| {
            state.pump_control();
            state.ports[index].write(src)
        })
    }

    fn poll(&self, index: usize) -> Poll {
        STATE.with(|state| {
            let port = &state.ports[index];
            Poll { readable: port.readable(), writable: port.writable() }
        })
    }

    fn nonblocking(&self) -> bool {
        true
    }
}

impl sysfs::Class for Ports {
    fn name(&self) -> &'static [u8] {
        b"virtio-ports"
    }

    fn count(&self) -> usize {
        port_count()
    }

    fn instance(&self, index: usize) -> Option<Name> {
        vport_name(Vport(index))
    }

    fn attributes(&self) -> &'static [&'static [u8]] {
        &[b"name"]
    }

    fn show(&self, index: usize, _: usize, out: &mut Content) -> Option<()> {
        STATE.with(|state| {
            out.push(until_nul(&state.ports()[index].name));
        });
        Some(())
    }
}

const F_MULTIPORT: u32 = 1 << 1;
const CONFIG_MAX_PORTS: u32 = 4;

const CONTROL_RECEIVE: u16 = 2;
const CONTROL_TRANSMIT: u16 = 3;

const DEVICE_READY: u16 = 0;
const DEVICE_ADD: u16 = 1;
const PORT_READY: u16 = 3;
const PORT_OPEN: u16 = 6;
const PORT_NAME: u16 = 7;

const QUEUE_SIZE: usize = 4;
const CONTROL_SIZE: u32 = 8;
const CONTROL_BUFFER: u32 = CONTROL_SIZE + 64;
const RECEIVE_BUFFERS: usize = 2;
const RECEIVE_BUFFER: u32 = 1024;
const TRANSMIT_BUFFER: usize = 16 * 1024;
const MAX_NAME: usize = 63;
const MAX_PORTS: usize = 8;
const PORT_PREFIX: &str = "vport0p";

static STATE: KernelCell<State> = KernelCell::new(State {
    device: None,
    port_count: 0,
    control_rx: Virtq::EMPTY,
    control_tx: Virtq::EMPTY,
    control_rx_buffers: [core::ptr::null_mut(); QUEUE_SIZE],
    control_tx_buffers: [Control { id: 0, event: 0, value: 0 }; QUEUE_SIZE],
    ports: [Port::EMPTY; MAX_PORTS],
});

pub fn port_count() -> usize {
    STATE.with(|state| {
        state.pump_control();
        state.port_count
    })
}

pub fn vport_name(port: Vport) -> Option<Name> {
    let named = STATE.with(|state| {
        state.pump_control();
        state.ports().get(port.0).is_some_and(|p| p.named)
    });
    named.then(|| {
        let mut name = Name::EMPTY;
        write!(name, "{PORT_PREFIX}{}", port.0);
        name
    })
}

pub fn signal_ready() -> bool {
    STATE.with(|state| state.ports().iter().any(Port::signal_due))
}

pub fn take_signal() -> Option<Vport> {
    STATE.with(|state| {
        let index = state.ports().iter().position(Port::signal_due)?;
        let port = &mut state.ports()[index];
        port.notified = true;
        port.blocked = false;
        Some(Vport(index))
    })
}

pub fn name_ptr(port: Vport) -> Option<*const c_char> {
    STATE.with(|state| state.ports().get(port.0).filter(|p| p.named).map(|p| p.name.as_ptr().cast()))
}

fn probe(device: Device) -> bool {
    let Some(features) = device.begin(F_MULTIPORT) else { return false };
    if features & F_MULTIPORT == 0 {
        return false;
    }
    STATE.with(|state| {
        if state.device.is_some() {
            return false;
        }
        if state.setup(device).is_ok() {
            return true;
        }
        state.device = None;
        state.port_count = 0;
        false
    })
}

fn post_receive(q: &mut Virtq, buffer: *mut u8, size: u32, by_head: &mut [*mut u8; QUEUE_SIZE]) {
    if let Some(head) = q.add(&[Buf { addr: buffer.cast(), len: size }], 0) {
        by_head[head as usize] = buffer;
    }
}

fn vport_index(name: &[u8]) -> Option<usize> {
    let digits =
        name.strip_prefix(PORT_PREFIX.as_bytes()).filter(|d| !d.is_empty() && d.iter().all(u8::is_ascii_digit))?;
    digits.iter().try_fold(0usize, |n, &c| n.checked_mul(10)?.checked_add((c - b'0') as usize))
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Control {
    id: u32,
    event: u16,
    value: u16,
}

struct Port {
    name: [u8; MAX_NAME + 1], // NUL-terminated
    named: bool,
    opened: bool,
    notified: bool,
    blocked: bool,
    rx: Virtq,
    tx: Virtq,
    rx_buffers: [*mut u8; QUEUE_SIZE],         // by descriptor head
    tx_buffers: [Option<HeapBuf>; QUEUE_SIZE], // by descriptor head, while the host reads them
    current: Option<u16>,
    current_length: u32,
    current_offset: u32,
}

impl Port {
    const EMPTY: Self = Self {
        name: [0; MAX_NAME + 1],
        named: false,
        opened: false,
        notified: false,
        blocked: false,
        rx: Virtq::EMPTY,
        tx: Virtq::EMPTY,
        rx_buffers: [core::ptr::null_mut(); QUEUE_SIZE],
        tx_buffers: [const { None }; QUEUE_SIZE],
        current: None,
        current_length: 0,
        current_offset: 0,
    };

    fn readable(&self) -> bool {
        self.current.is_some() || self.rx.has_used()
    }

    fn writable(&self) -> bool {
        self.tx.free_count > 0 || self.tx.has_used()
    }

    fn signal_due(&self) -> bool {
        self.opened && ((!self.notified && self.readable()) || (self.blocked && self.writable()))
    }

    fn open(&mut self) -> core::result::Result<(), ()> {
        let buffers = [(); RECEIVE_BUFFERS].map(|_| malloc(RECEIVE_BUFFER as usize).cast::<u8>());
        if buffers.iter().any(|b| b.is_null()) {
            buffers.iter().for_each(|&b| unsafe { free(b.cast()) });
            return Err(());
        }
        for buffer in buffers {
            post_receive(&mut self.rx, buffer, RECEIVE_BUFFER, &mut self.rx_buffers);
        }
        self.rx.notify();
        self.opened = true;
        Ok(())
    }

    fn read(&mut self, dst: &mut [u8]) -> usize {
        let mut done = 0;
        while done < dst.len() {
            let head = match self.current {
                Some(head) => head,
                None => {
                    let Some((head, length)) = self.rx.next_used() else { break };
                    self.current = Some(head);
                    self.current_length = length.min(RECEIVE_BUFFER);
                    self.current_offset = 0;
                    head
                }
            };
            let buffer = self.rx_buffers[head as usize];
            let chunk = ((self.current_length - self.current_offset) as usize).min(dst.len() - done);
            let src = unsafe { core::slice::from_raw_parts(buffer.add(self.current_offset as usize), chunk) };
            dst[done..done + chunk].copy_from_slice(src);
            done += chunk;
            self.current_offset += chunk as u32;
            if self.current_offset >= self.current_length {
                self.current = None;
                post_receive(&mut self.rx, buffer, RECEIVE_BUFFER, &mut self.rx_buffers);
                self.rx.notify();
            }
        }
        done
    }

    fn reclaim(&mut self) {
        while let Some((head, _)) = self.tx.next_used() {
            self.tx_buffers[head as usize] = None;
        }
    }

    fn write(&mut self, src: &[u8]) -> Result<usize> {
        self.reclaim();
        if self.tx.free_count == 0 {
            self.blocked = true;
            return Err(Error::Again);
        }
        let src = &src[..src.len().min(TRANSMIT_BUFFER)];
        let mut buffer = HeapBuf::new(src.len()).ok_or(Error::NoSpace)?;
        buffer.copy_from_slice(src);
        let buf = Buf { addr: buffer.as_ptr().cast(), len: src.len() as u32 };
        let head = self.tx.add(&[buf], 1).ok_or(Error::Again)?;
        self.tx_buffers[head as usize] = Some(buffer);
        self.tx.notify();
        Ok(src.len())
    }
}

struct State {
    device: Option<Device>,
    port_count: usize,
    control_rx: Virtq,
    control_tx: Virtq,
    control_rx_buffers: [*mut u8; QUEUE_SIZE],
    control_tx_buffers: [Control; QUEUE_SIZE],
    ports: [Port; MAX_PORTS],
}

impl State {
    fn ports(&mut self) -> &mut [Port] {
        &mut self.ports[..self.port_count]
    }

    fn send_control(&mut self, id: u32, event: u16, value: u16) {
        while self.control_tx.next_used().is_some() {}
        if self.control_tx.free_count == 0 {
            return;
        }
        let message = &mut self.control_tx_buffers[self.control_tx.free_head() as usize];
        *message = Control { id, event, value };
        let buf = Buf { addr: (message as *const Control).cast(), len: CONTROL_SIZE };
        if self.control_tx.add(&[buf], 1).is_some() {
            self.control_tx.notify();
        }
    }

    fn pump_control(&mut self) {
        if self.device.is_none() {
            return;
        }
        loop {
            let mut reposted = false;
            while let Some((head, length)) = self.control_rx.next_used() {
                let buffer = self.control_rx_buffers[head as usize];
                if length >= CONTROL_SIZE {
                    let message = unsafe { buffer.cast::<Control>().read_unaligned() };
                    if (message.id as usize) < self.port_count {
                        match message.event {
                            DEVICE_ADD => self.send_control(message.id, PORT_READY, 1),
                            PORT_NAME => {
                                let length = ((length - CONTROL_SIZE) as usize).min(MAX_NAME);
                                let port = &mut self.ports[message.id as usize];
                                let name =
                                    unsafe { core::slice::from_raw_parts(buffer.add(CONTROL_SIZE as usize), length) };
                                port.name[..length].copy_from_slice(name);
                                port.name[length] = 0;
                                port.named = true;
                            }
                            _ => {}
                        }
                    }
                }
                post_receive(&mut self.control_rx, buffer, CONTROL_BUFFER, &mut self.control_rx_buffers);
                reposted = true;
            }
            if !reposted {
                return;
            }
            self.control_rx.notify();
        }
    }

    fn find(&mut self, name: &[u8]) -> Option<usize> {
        self.pump_control();
        let index = vport_index(name).or_else(|| self.ports().iter().position(|p| until_nul(&p.name) == name))?;
        self.ports().get(index).filter(|p| p.named).map(|_| index)
    }

    fn setup(&mut self, device: Device) -> core::result::Result<(), ()> {
        self.device = Some(device);
        self.port_count = (device.config32(CONFIG_MAX_PORTS) as usize).min(MAX_PORTS);
        self.control_rx.init(device, CONTROL_RECEIVE, QUEUE_SIZE as u16)?;
        self.control_tx.init(device, CONTROL_TRANSMIT, QUEUE_SIZE as u16)?;
        for (index, port) in self.ports().iter_mut().enumerate() {
            let (rx, tx) = if index == 0 { (0, 1) } else { (2 * index as u16 + 2, 2 * index as u16 + 3) };
            port.rx.init(device, rx, QUEUE_SIZE as u16)?;
            port.tx.init(device, tx, QUEUE_SIZE as u16)?;
        }
        for _ in 0..QUEUE_SIZE {
            let buffer = malloc(CONTROL_BUFFER as usize).cast::<u8>();
            if buffer.is_null() {
                return Err(());
            }
            post_receive(&mut self.control_rx, buffer, CONTROL_BUFFER, &mut self.control_rx_buffers);
        }
        device.ready();
        self.send_control(0, DEVICE_READY, 1);
        Ok(())
    }
}
