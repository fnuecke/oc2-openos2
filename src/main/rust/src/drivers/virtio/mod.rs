// SPDX-License-Identifier: MIT
//! virtio-mmio v2 transport and split virtqueues, shared by all virtio drivers. Probing a
//! virtio,mmio node hands the device to the driver for its device ID, see drivers::VIRTIO_DRIVERS.

pub mod blk;
pub mod console;
pub mod snd;

use super::plic::HandlerData;
use super::{VIRTIO_DRIVERS, plic};
use crate::arch::{IrqCell, Mmio};
use crate::board::virt_to_phys;
use crate::fdt::Node;
use crate::mem::malloc;
use core::arch::asm;
use core::ffi::c_void;

pub const DRIVER: super::Driver = super::Driver { compatible: &[b"virtio,mmio"], probe };

pub struct Driver {
    pub id: u32,
    pub probe: fn(Device) -> bool,
    /// Leaves a configuration change pending until the driver acknowledges it, see
    /// Device::ack_config_change. Sedna fails block requests while a change is pending,
    /// which avoids old ones getting applied to a new medium (e.g. floppy changed).
    pub keeps_config_change: bool,
}

const REG_MAGIC: u64 = 0x000;
const REG_VERSION: u64 = 0x004;
const REG_DEVICE_ID: u64 = 0x008;
const REG_DEVICE_FEATURES: u64 = 0x010;
const REG_DEVICE_FEATURES_SEL: u64 = 0x014;
const REG_DRIVER_FEATURES: u64 = 0x020;
const REG_DRIVER_FEATURES_SEL: u64 = 0x024;
const REG_QUEUE_SEL: u64 = 0x030;
const REG_QUEUE_SIZE_MAX: u64 = 0x034;
const REG_QUEUE_SIZE: u64 = 0x038;
const REG_QUEUE_READY: u64 = 0x044;
const REG_QUEUE_NOTIFY: u64 = 0x050;
const REG_INTERRUPT_STATUS: u64 = 0x060;
const REG_INTERRUPT_ACK: u64 = 0x064;
const REG_STATUS: u64 = 0x070;
const REG_QUEUE_DESC_LOW: u64 = 0x080;
const REG_QUEUE_DESC_HIGH: u64 = 0x084;
const REG_QUEUE_DRIVER_LOW: u64 = 0x090;
const REG_QUEUE_DRIVER_HIGH: u64 = 0x094;
const REG_QUEUE_DEVICE_LOW: u64 = 0x0A0;
const REG_QUEUE_DEVICE_HIGH: u64 = 0x0A4;
const REG_CONFIG: u64 = 0x100;

const MAGIC: u32 = 0x74726976;

const STATUS_ACKNOWLEDGE: u32 = 1;
const STATUS_DRIVER: u32 = 2;
const STATUS_DRIVER_OK: u32 = 4;
const STATUS_FEATURES_OK: u32 = 8;
const STATUS_DEVICE_NEEDS_RESET: u32 = 64;

const F_VERSION_1: u32 = 1 << 0; // feature bit 32

const INTERRUPT_USED_BUFFER: u32 = 1;
const INTERRUPT_CONFIG_CHANGE: u32 = 2;

const DESC_F_NEXT: u16 = 1;

const MAX_DEVICES: usize = 16;
static LINES: IrqCell<[Option<Line>; MAX_DEVICES]> = IrqCell::new([None; MAX_DEVICES]);
const DESC_F_WRITE: u16 = 2;

#[derive(Clone, Copy)]
pub struct Device(Mmio);

impl Device {
    pub unsafe fn new(base: u64) -> Self {
        Self(unsafe { Mmio::new(base) })
    }

    pub fn base(self) -> u64 {
        self.0.base()
    }

    pub fn id(self) -> Option<u32> {
        if self.0.read32(REG_MAGIC) != MAGIC || self.0.read32(REG_VERSION) != 2 {
            return None;
        }
        Some(self.0.read32(REG_DEVICE_ID)).filter(|&id| id != 0)
    }

    pub fn begin(self, features: u32) -> Option<u32> {
        let r = self.0;
        r.write32(REG_STATUS, 0);
        r.write32(REG_STATUS, STATUS_ACKNOWLEDGE);
        r.write32(REG_STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER);

        r.write32(REG_DEVICE_FEATURES_SEL, 1);
        if r.read32(REG_DEVICE_FEATURES) & F_VERSION_1 == 0 {
            self.reset();
            return None;
        }
        r.write32(REG_DEVICE_FEATURES_SEL, 0);
        let offered = r.read32(REG_DEVICE_FEATURES);

        r.write32(REG_DRIVER_FEATURES_SEL, 1);
        r.write32(REG_DRIVER_FEATURES, F_VERSION_1);
        r.write32(REG_DRIVER_FEATURES_SEL, 0);
        r.write32(REG_DRIVER_FEATURES, offered & features);

        r.write32(REG_STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK);
        if r.read32(REG_STATUS) & STATUS_FEATURES_OK == 0 {
            self.reset();
            return None;
        }
        Some(offered & features)
    }

    pub fn ready(self) {
        self.0.write32(REG_STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK);
    }

    pub fn reset(self) {
        self.0.write32(REG_STATUS, 0);
        while self.0.read32(REG_STATUS) != 0 {}
    }

    pub fn config8(self, offset: u32) -> u8 {
        self.0.read8(REG_CONFIG + offset as u64)
    }

    pub fn config32(self, offset: u32) -> u32 {
        self.0.read32(REG_CONFIG + offset as u64)
    }

    fn interrupt_status(self) -> u32 {
        self.0.read32(REG_INTERRUPT_STATUS)
    }

    fn ack_interrupt(self, status: u32) {
        self.0.write32(REG_INTERRUPT_ACK, status);
    }

    pub fn config_changed(self) -> bool {
        self.interrupt_status() & INTERRUPT_CONFIG_CHANGE != 0
    }

    pub fn ack_config_change(self) {
        self.ack_interrupt(INTERRUPT_CONFIG_CHANGE);
        if let Some(line) = line(self.0.base()) {
            plic::set_enabled(line.irq, true);
        }
    }

    fn select_queue(self, index: u16) -> u32 {
        self.0.write32(REG_QUEUE_SEL, index as u32);
        self.0.read32(REG_QUEUE_SIZE_MAX)
    }

    fn set_queue(self, size: u16, desc: u64, avail: u64, used: u64) {
        let r = self.0;
        r.write32(REG_QUEUE_SIZE, size as u32);
        r.write32(REG_QUEUE_DESC_LOW, desc as u32);
        r.write32(REG_QUEUE_DESC_HIGH, (desc >> 32) as u32);
        r.write32(REG_QUEUE_DRIVER_LOW, avail as u32);
        r.write32(REG_QUEUE_DRIVER_HIGH, (avail >> 32) as u32);
        r.write32(REG_QUEUE_DEVICE_LOW, used as u32);
        r.write32(REG_QUEUE_DEVICE_HIGH, (used >> 32) as u32);
        r.write32(REG_QUEUE_READY, 1);
    }

    fn notify(self, queue: u16) {
        self.0.write32(REG_QUEUE_NOTIFY, queue as u32);
    }

    pub fn failed(self) -> bool {
        self.0.read32(REG_STATUS) & STATUS_DEVICE_NEEDS_RESET != 0
    }
}

#[repr(C)]
struct Desc {
    addr: u64,
    len: u32,
    flags: u16,
    next: u16,
}

#[repr(C)]
struct Avail {
    flags: u16,
    idx: u16,
    ring: [u16; 0],
}

#[repr(C)]
struct UsedElem {
    id: u32,
    len: u32,
}

#[repr(C)]
struct Used {
    flags: u16,
    idx: u16,
    ring: [UsedElem; 0],
}

pub struct Virtq {
    base: u64,
    index: u16,
    size: u16,
    free_head: u16,
    pub free_count: u16,
    last_used: u16,
    desc: *mut Desc,
    avail: *mut Avail,
    used: *mut Used,
}

pub struct Buf {
    pub addr: *const c_void,
    pub len: u32,
}

impl Virtq {
    pub const EMPTY: Self = Self {
        base: 0,
        index: 0,
        size: 0,
        free_head: 0,
        free_count: 0,
        last_used: 0,
        desc: core::ptr::null_mut(),
        avail: core::ptr::null_mut(),
        used: core::ptr::null_mut(),
    };

    pub fn init(&mut self, device: Device, index: u16, mut size: u16) -> Result<(), ()> {
        let max = device.select_queue(index);
        if max == 0 {
            return Err(());
        }
        size = (size as u32).min(max) as u16;

        let desc_bytes = 16 * size as usize;
        let avail_bytes = 6 + 2 * size as usize;
        let used_offset = (desc_bytes + avail_bytes + 3) & !3;
        let used_bytes = 6 + 8 * size as usize;
        let memory = malloc(used_offset + used_bytes + 15).cast::<u8>();
        if memory.is_null() {
            return Err(());
        }
        let memory = unsafe { memory.add(memory.align_offset(16)) };
        unsafe { memory.write_bytes(0, used_offset + used_bytes) };

        self.base = device.base();
        self.index = index;
        self.size = size;
        self.desc = memory.cast();
        self.avail = unsafe { memory.add(desc_bytes) }.cast();
        self.used = unsafe { memory.add(used_offset) }.cast();
        self.last_used = 0;
        for i in 0..size {
            unsafe { (*self.desc.add(i as usize)).next = i + 1 };
        }
        self.free_head = 0;
        self.free_count = size;

        let (desc, avail, used) = (self.desc as u64, self.avail as u64, self.used as u64);
        device.set_queue(size, virt_to_phys(desc), virt_to_phys(avail), virt_to_phys(used));
        Ok(())
    }

    pub fn device(&self) -> Device {
        Device(unsafe { Mmio::new(self.base) })
    }

    pub fn free_head(&self) -> u16 {
        self.free_head
    }

    /// Posts a chain of `out` device-readable then device-writable buffers. Returns its head.
    pub fn add(&mut self, bufs: &[Buf], out: usize) -> Option<u16> {
        let count = bufs.len();
        if count == 0 || count > self.free_count as usize {
            return None;
        }
        let head = self.free_head;
        let mut last = head;
        let mut index = head;
        for (i, buf) in bufs.iter().enumerate() {
            let desc = unsafe { &mut *self.desc.add(index as usize) };
            desc.addr = virt_to_phys(buf.addr as u64);
            desc.len = buf.len;
            desc.flags = if i < out { 0 } else { DESC_F_WRITE } | if i + 1 < count { DESC_F_NEXT } else { 0 };
            last = index;
            index = desc.next;
        }
        self.free_head = unsafe { (*self.desc.add(last as usize)).next };
        self.free_count -= count as u16;

        unsafe {
            let idx = (*self.avail).idx;
            (&raw mut (*self.avail).ring).cast::<u16>().add((idx % self.size) as usize).write(head);
            asm!("fence w, w");
            (&raw mut (*self.avail).idx).write_volatile(idx.wrapping_add(1));
        }
        Some(head)
    }

    pub fn notify(&self) {
        unsafe { asm!("fence w, o") };
        self.device().notify(self.index);
    }

    pub fn has_used(&self) -> bool {
        unsafe { (&raw const (*self.used).idx).read_volatile() != self.last_used }
    }

    pub fn next_used(&mut self) -> Option<(u16, u32)> {
        if !self.has_used() {
            return None;
        }
        unsafe { asm!("fence r, r") };
        let elem = unsafe {
            (&raw const (*self.used).ring).cast::<UsedElem>().add((self.last_used % self.size) as usize).read_volatile()
        };
        self.last_used = self.last_used.wrapping_add(1);
        if elem.id >= self.size as u32 {
            return None;
        }
        let head = elem.id as u16;

        let mut index = head;
        let mut count = 1;
        unsafe {
            while (*self.desc.add(index as usize)).flags & DESC_F_NEXT != 0 {
                index = (*self.desc.add(index as usize)).next;
                count += 1;
            }
            (*self.desc.add(index as usize)).next = self.free_head;
        }
        self.free_head = head;
        self.free_count += count;

        let device = self.device();
        if device.interrupt_status() & INTERRUPT_USED_BUFFER != 0 {
            device.ack_interrupt(INTERRUPT_USED_BUFFER);
        }
        Some((head, elem.len))
    }
}

fn probe(node: &Node) {
    let Some((base, _)) = node.reg() else { return };
    let device = unsafe { Device::new(base) };
    let Some(driver) = device.id().and_then(|id| VIRTIO_DRIVERS.iter().find(|d| d.id == id)) else { return };
    // Before probing, which may wait for the device.
    if let Some(irq) = node.irq() {
        let line = Line { base, irq, keeps_config_change: driver.keeps_config_change };
        let added = LINES.with(|lines| lines.iter_mut().find(|l| l.is_none()).map(|free| *free = Some(line)).is_some());
        if added {
            plic::register(irq, interrupt, HandlerData(base as usize));
        }
    }
    if !(driver.probe)(device) {
        device.reset();
    }
}

fn interrupt(HandlerData(base): HandlerData) {
    let device = unsafe { Device::new(base as u64) };
    let Some(line) = line(base as u64) else { return };
    let mut status = device.interrupt_status();
    if line.keeps_config_change && status & INTERRUPT_CONFIG_CHANGE != 0 {
        // The pending change keeps the line raised, so it stays masked until acknowledged.
        plic::set_enabled(line.irq, false);
        status &= !INTERRUPT_CONFIG_CHANGE;
    }
    device.ack_interrupt(status);
}

fn line(base: u64) -> Option<Line> {
    LINES.with(|lines| lines.iter().flatten().find(|line| line.base == base).copied())
}

#[derive(Clone, Copy)]
struct Line {
    base: u64,
    irq: u32,
    keeps_config_change: bool,
}
