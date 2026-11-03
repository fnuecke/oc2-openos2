// SPDX-License-Identifier: MIT

use super::{Buf, Device, Driver, Virtq};
use crate::arch::{KernelCell, time_now};
use crate::mem::HeapBox;
use crate::time::{frequency, wait};

pub const DRIVER: Driver = Driver { id: 2, probe, keeps_config_change: true, interrupt_hook: None };
pub const SECTOR_SIZE: usize = 512;
pub const MAX_DISKS: usize = 16;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Fault {
    Io,
    Changed,
}

#[derive(Clone, Copy)]
pub struct Disk(*mut State);

impl Disk {
    pub fn sectors(self) -> u64 {
        unsafe { (*self.0).capacity }
    }

    pub fn ack_change(self) {
        let state = unsafe { &mut *self.0 };
        let device = state.queue.device();
        if device.config_changed() {
            device.ack_config_change();
            state.capacity = read_capacity(device);
        }
    }

    pub fn change_pending(self) -> bool {
        unsafe { (*self.0).queue.device().config_changed() }
    }

    pub fn read_only(self) -> bool {
        unsafe { (*self.0).read_only }
    }

    pub fn read(self, sector: u64, dst: &mut [u8]) -> Result<(), Fault> {
        self.transfer(REQUEST_IN, sector, dst.as_mut_ptr(), dst.len())
    }

    pub fn write(self, sector: u64, src: &[u8]) -> Result<(), Fault> {
        self.transfer(REQUEST_OUT, sector, src.as_ptr().cast_mut(), src.len())
    }

    fn transfer(self, kind: u32, sector: u64, data: *mut u8, len: usize) -> Result<(), Fault> {
        let state = self.0;
        let (queue, request, status, dead) =
            unsafe { (&mut (*state).queue, &raw mut (*state).request, &raw mut (*state).status, &mut (*state).dead) };
        let device = queue.device();
        if *dead {
            return Err(Fault::Io);
        }
        if device.config_changed() {
            return Err(Fault::Changed);
        }
        if len == 0 || !len.is_multiple_of(SECTOR_SIZE) || sector + (len / SECTOR_SIZE) as u64 > self.sectors() {
            return Err(Fault::Io);
        }
        unsafe {
            request.write_volatile(Request { kind, reserved: 0, sector });
            status.write_volatile(0xFF);
        }

        let bufs = [
            Buf { addr: request.cast(), len: size_of::<Request>() as u32 },
            Buf { addr: data.cast(), len: len as u32 },
            Buf { addr: status.cast(), len: 1 },
        ];
        let out = if kind == REQUEST_OUT { 2 } else { 1 };
        queue.add(&bufs, out).ok_or(Fault::Io)?;
        queue.notify();
        let deadline = time_now() + TIMEOUT_SECONDS * frequency();
        wait(|| queue.has_used() || device.failed() || device.config_changed(), deadline);
        while !queue.has_used() && !device.failed() && time_now() < deadline {}
        if device.failed() || queue.next_used().is_none() {
            device.reset();
            *dead = true;
            return Err(Fault::Io);
        }
        match unsafe { status.read_volatile() } {
            0 => Ok(()),
            _ if device.config_changed() => Err(Fault::Changed),
            _ => Err(Fault::Io),
        }
    }
}

const REQUEST_IN: u32 = 0;
const REQUEST_OUT: u32 = 1;
const QUEUE_SIZE: u16 = 4;
const F_RO: u32 = 1 << 5;
const TIMEOUT_SECONDS: u64 = 10;
static DISKS: KernelCell<[Option<Disk>; MAX_DISKS]> = KernelCell::new([None; MAX_DISKS]);

pub fn count() -> usize {
    DISKS.with(|disks| disks.iter().take_while(|d| d.is_some()).count())
}

pub fn get(index: usize) -> Option<Disk> {
    DISKS.with(|disks| disks.get(index).copied().flatten())
}

fn probe(device: Device) -> bool {
    if count() == MAX_DISKS {
        return false;
    }
    let Some(features) = device.begin(F_RO) else { return false };
    let Some(mut state) = (unsafe { HeapBox::<State>::zeroed() }) else { return false };
    if state.queue.init(device, 0, QUEUE_SIZE).is_err() {
        return false;
    }
    device.ready();
    state.read_only = features & F_RO != 0;
    state.capacity = read_capacity(device);
    let index = count();
    DISKS.with(|disks| disks[index] = Some(Disk(HeapBox::forget(state))));
    true
}

fn read_capacity(device: Device) -> u64 {
    device.config32(0) as u64 | (device.config32(4) as u64) << 32
}

#[repr(C)]
struct Request {
    kind: u32,
    reserved: u32,
    sector: u64,
}

struct State {
    queue: Virtq,
    capacity: u64,
    read_only: bool,
    dead: bool, // the device failed and was reset
    request: Request,
    status: u8,
}
