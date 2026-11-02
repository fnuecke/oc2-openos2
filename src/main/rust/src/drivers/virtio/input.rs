// SPDX-License-Identifier: MIT

use super::{Buf, Device, Driver, Virtq};
use crate::api::time::TIME_NEVER;
use crate::arch::{IrqCell, time_now};
use crate::fs::Name;
use crate::fs::sysfs::{self, Content};
use crate::mem::HeapBuf;
use crate::time::{entropy_add, frequency};

pub const DRIVER: Driver = Driver { id: 18, probe, keeps_config_change: false, interrupt_hook: Some(interrupt) };

pub struct Key {
    pub keyboard: Name,
    pub down: bool,
    pub ch: u32,
    pub code: u16,
}

pub struct Keyboards;

impl sysfs::Class for Keyboards {
    fn name(&self) -> &'static [u8] {
        b"input"
    }

    fn count(&self) -> usize {
        count()
    }

    fn instance(&self, index: usize) -> Option<Name> {
        name(index)
    }

    fn attributes(&self) -> &'static [&'static [u8]] {
        &[b"name"]
    }

    fn show(&self, index: usize, _: usize, out: &mut Content) -> Option<()> {
        let device = STATE.with(|state| state.active().get(index).map(|k| k.events.device()))?;
        device.set_config8(CONFIG_SELECT, SELECT_ID_NAME);
        device.set_config8(CONFIG_SUBSEL, 0);
        for offset in 0..device.config8(CONFIG_SIZE) as u32 {
            out.push(&[device.config8(CONFIG_DATA + offset)]);
        }
        Some(())
    }
}

const CONFIG_SELECT: u32 = 0;
const CONFIG_SUBSEL: u32 = 1;
const CONFIG_SIZE: u32 = 2;
const CONFIG_DATA: u32 = 8;
const SELECT_ID_NAME: u8 = 0x01;
const SELECT_EV_BITS: u8 = 0x11;

const EV_KEY: u16 = 1;
const EVENT_QUEUE: u16 = 0;
const EVENT_SIZE: usize = 8;
const MAX_BUFFERS: u16 = 16;

const KEY_LEFTCTRL: u16 = 29;
const KEY_LEFTSHIFT: u16 = 42;
const KEY_RIGHTSHIFT: u16 = 54;
const KEY_LEFTALT: u16 = 56;
const KEY_CAPSLOCK: u16 = 58;
const KEY_NUMLOCK: u16 = 69;
const KEY_SCROLLLOCK: u16 = 70;
const KEY_RIGHTCTRL: u16 = 97;
const KEY_RIGHTALT: u16 = 100;

const SHIFT: u8 = 1;
const CTRL: u8 = 2;
const CAPS: u8 = 4;
const LEFT_SHIFT: u8 = 8;
const RIGHT_SHIFT: u8 = 16;
const LEFT_CTRL: u8 = 32;
const RIGHT_CTRL: u8 = 64;

const REPEAT_DELAY_MS: u64 = 250;
const REPEAT_PERIOD_MS: u64 = 33;

const MAX_KEYBOARDS: usize = 4;
const RING_SIZE: usize = 32;
const TRACKED_KEYS: usize = 256;

// Linux US defkeymap, Num Lock on, codes up to KEY_KPSLASH.
const PLAIN: &[u8; 99] = b"\0\x1b1234567890-=\x7f\tqwertyuiop[]\r\0asdfghjkl;'`\0\\zxcvbnm,./\0*\0 \0\
    \0\0\0\0\0\0\0\0\0\0\0\x00789-456+1230.\0\0<\0\0\0\0\0\0\0\0\0\r\0/";
const SHIFTED: &[u8; 99] = b"\0\x1b!@#$%^&*()_+\x7f\tQWERTYUIOP{}\r\0ASDFGHJKL:\"~\0|ZXCVBNM<>?\0*\0 \0\
    \0\0\0\0\0\0\0\0\0\0\0\x00789-456+1230.\0\0>\0\0\0\0\0\0\0\0\0\r\0/";
const CONTROL: &[u8; 99] =
    b"\0\0\0\0\x1b\x1c\x1d\x1e\x1f\x7f\0\0\x1f\0\x08\0\x11\x17\x05\x12\x14\x19\x15\t\x0f\x10\x1b\x1d\r\0\
    \x01\x13\x04\x06\x07\x08\x0a\x0b\x0c\0\x07\0\0\x1c\x1a\x18\x03\x16\x02\x0e\r\0\0\x7f\0*\0\0\0\0\0\0\0\0\0\0\0\0\0\0\
    \x00789-456+1230.\0\0\0\0\0\0\0\0\0\0\0\0\r\0/";
const SHIFTED_CONTROL: &[u8; 99] = b"\0\0\0\0\0\0\0\0\0\0\0\0\x1f\0\0\0\x11\x17\x05\x12\x14\x19\x15\t\x0f\x10\0\0\r\0\
    \x01\x13\x04\x06\x07\x08\x0a\x0b\x0c\0\0\0\0\0\x1a\x18\x03\x16\x02\x0e\r\0\0\0\0*\0\0\0\0\0\0\0\0\0\0\0\0\0\0\
    \x00789-456+1230.\0\0\0\0\0\0\0\0\0\0\0\0\r\0/";

static STATE: IrqCell<State> = IrqCell::new(State { count: 0, keyboards: [const { Keyboard::EMPTY }; MAX_KEYBOARDS] });

pub fn count() -> usize {
    STATE.with(|state| state.count)
}

pub fn name(index: usize) -> Option<Name> {
    (index < count()).then(|| input_name(index))
}

pub fn signal_ready() -> bool {
    let now = time_now();
    STATE.with(|state| state.active().iter().any(|k| k.ready(now)))
}

pub fn take_signal() -> Option<Key> {
    let now = time_now();
    STATE.with(|state| state.active().iter_mut().enumerate().find_map(|(index, k)| Some(k.take(now)?.key(index))))
}

pub fn repeat_deadline() -> u64 {
    let due = |k: &Keyboard| k.repeat.map_or(TIME_NEVER, |r| r.at);
    STATE.with(|state| state.active().iter().map(due).min().unwrap_or(TIME_NEVER))
}

fn probe(device: Device) -> bool {
    if device.begin(0).is_none() {
        return false;
    }
    device.set_config8(CONFIG_SELECT, SELECT_EV_BITS);
    device.set_config8(CONFIG_SUBSEL, EV_KEY as u8);
    if device.config8(CONFIG_SIZE) == 0 {
        return false;
    }
    STATE.with(|state| {
        let Some(keyboard) = state.keyboards.get_mut(state.count) else { return false };
        if keyboard.events.init(device, EVENT_QUEUE, MAX_BUFFERS).is_err() {
            return false;
        }
        let Some(buffers) = HeapBuf::zeroed(keyboard.events.free_count as usize * EVENT_SIZE) else { return false };
        keyboard.buffers = buffers;
        device.ready();
        while keyboard.events.free_count > 0 && keyboard.post() {}
        keyboard.events.notify();
        state.count += 1;
        true
    })
}

fn interrupt(device: Device) {
    let now = time_now();
    STATE.with(|state| {
        if let Some(keyboard) = state.active().iter_mut().find(|k| k.events.device().base() == device.base()) {
            keyboard.drain(now);
        }
    });
}

fn input_name(index: usize) -> Name {
    let mut name = Name::EMPTY;
    write!(name, "input{index}");
    name
}

fn ticks(ms: u64) -> u64 {
    frequency() * ms / 1000
}

fn repeats(code: u16) -> bool {
    !matches!(
        code,
        KEY_LEFTCTRL
            | KEY_RIGHTCTRL
            | KEY_LEFTSHIFT
            | KEY_RIGHTSHIFT
            | KEY_LEFTALT
            | KEY_RIGHTALT
            | KEY_CAPSLOCK
            | KEY_NUMLOCK
            | KEY_SCROLLLOCK
    )
}

fn character(code: u16, modifiers: u8) -> u32 {
    let index = code as usize;
    let letter = PLAIN.get(index).is_some_and(u8::is_ascii_lowercase);
    let shift = (modifiers & SHIFT != 0) != (letter && modifiers & CAPS != 0);
    let map = match (modifiers & CTRL != 0, shift) {
        (false, false) => PLAIN,
        (false, true) => SHIFTED,
        (true, false) => CONTROL,
        (true, true) => SHIFTED_CONTROL,
    };
    map.get(index).copied().unwrap_or(0) as u32
}

#[derive(Clone, Copy)]
struct Entry {
    code: u16,
    down: bool,
    modifiers: u8,
}

impl Entry {
    const EMPTY: Self = Self { code: 0, down: false, modifiers: 0 };

    fn key(self, index: usize) -> Key {
        Key { keyboard: input_name(index), down: self.down, ch: character(self.code, self.modifiers), code: self.code }
    }
}

#[derive(Clone, Copy)]
struct Repeat {
    code: u16,
    at: u64,
}

struct State {
    count: usize,
    keyboards: [Keyboard; MAX_KEYBOARDS],
}

impl State {
    fn active(&mut self) -> &mut [Keyboard] {
        self.keyboards.get_mut(..self.count).unwrap_or_default()
    }
}

struct Keyboard {
    events: Virtq,
    buffers: HeapBuf, // EVENT_SIZE bytes per descriptor, by descriptor index
    ring: [Entry; RING_SIZE],
    head: usize,
    tail: usize,
    modifiers: u8,
    repeat: Option<Repeat>,
    pressed: [u8; TRACKED_KEYS / 8],
}

impl Keyboard {
    const EMPTY: Self = Self {
        events: Virtq::EMPTY,
        buffers: HeapBuf::EMPTY,
        ring: [Entry::EMPTY; RING_SIZE],
        head: 0,
        tail: 0,
        modifiers: 0,
        repeat: None,
        pressed: [0; TRACKED_KEYS / 8],
    };

    fn post(&mut self) -> bool {
        let head = self.events.free_head();
        let at = head as usize * EVENT_SIZE;
        let Some(buffer) = self.buffers.get_mut(at..at + EVENT_SIZE) else { return false };
        let buf = Buf { addr: buffer.as_ptr().cast(), len: EVENT_SIZE as u32 };
        self.events.add(&[buf], 0) == Some(head)
    }

    fn drain(&mut self, now: u64) {
        let mut posted = false;
        while let Some((head, len)) = self.events.next_used() {
            let at = head as usize * EVENT_SIZE;
            let event = self.buffers.get(at..at + EVENT_SIZE).and_then(|e| <[u8; EVENT_SIZE]>::try_from(e).ok());
            if let Some([t0, t1, c0, c1, v0, v1, v2, v3]) = event.filter(|_| len as usize >= EVENT_SIZE)
                && u16::from_le_bytes([t0, t1]) == EV_KEY
            {
                self.handle(u16::from_le_bytes([c0, c1]), u32::from_le_bytes([v0, v1, v2, v3]), now);
            }
            posted |= self.post();
        }
        if posted {
            self.events.notify();
        }
    }

    fn handle(&mut self, code: u16, value: u32, now: u64) {
        let down = match value {
            0 => false,
            1 => true,
            _ => return,
        };
        if let Some(bits) = self.pressed.get_mut(code as usize / 8) {
            let bit = 1 << (code % 8);
            if (*bits & bit != 0) == down {
                return;
            }
            *bits ^= bit;
        }
        entropy_add(now ^ code as u64);
        let held = match code {
            KEY_LEFTSHIFT => LEFT_SHIFT,
            KEY_RIGHTSHIFT => RIGHT_SHIFT,
            KEY_LEFTCTRL => LEFT_CTRL,
            KEY_RIGHTCTRL => RIGHT_CTRL,
            _ => 0,
        };
        if down {
            self.modifiers |= held;
        } else {
            self.modifiers &= !held;
        }
        if code == KEY_CAPSLOCK && down {
            self.modifiers ^= CAPS;
        }
        self.repeat = (down && repeats(code)).then(|| Repeat { code, at: now + ticks(REPEAT_DELAY_MS) });
        self.push(Entry { code, down, modifiers: self.state() });
    }

    fn state(&self) -> u8 {
        let shift = if self.modifiers & (LEFT_SHIFT | RIGHT_SHIFT) != 0 { SHIFT } else { 0 };
        let ctrl = if self.modifiers & (LEFT_CTRL | RIGHT_CTRL) != 0 { CTRL } else { 0 };
        shift | ctrl | (self.modifiers & CAPS)
    }

    fn push(&mut self, entry: Entry) {
        if self.head.wrapping_sub(self.tail) == RING_SIZE {
            self.tail = self.tail.wrapping_add(1);
        }
        if let Some(slot) = self.ring.get_mut(self.head % RING_SIZE) {
            *slot = entry;
        }
        self.head = self.head.wrapping_add(1);
    }

    fn ready(&self, now: u64) -> bool {
        self.head != self.tail || self.repeat.is_some_and(|r| now >= r.at)
    }

    fn take(&mut self, now: u64) -> Option<Entry> {
        if self.head != self.tail {
            let entry = self.ring.get(self.tail % RING_SIZE).copied();
            self.tail = self.tail.wrapping_add(1);
            return entry;
        }
        let modifiers = self.state();
        let repeat = self.repeat.as_mut().filter(|r| now >= r.at)?;
        repeat.at = now + ticks(REPEAT_PERIOD_MS);
        Some(Entry { code: repeat.code, down: true, modifiers })
    }
}
