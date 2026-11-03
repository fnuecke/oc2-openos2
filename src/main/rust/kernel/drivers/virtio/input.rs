// SPDX-License-Identifier: MIT

use super::{Buf, Device, VDriver, VIRTIO_DRIVERS, Virtq};
use crate::api::time::TIME_NEVER;
use crate::arch::{IrqCell, KernelCell, time_now};
use crate::fs::devfs::{self, DEVICES};
use crate::fs::sysfs::{self, CLASSES, Content};
use crate::fs::{Error, Name, OpenFlags, Result};
use crate::mem::HeapBuf;
use crate::signal::{self, Event, SOURCES};
use crate::time::{entropy_add, frequency};

register!(VIRTIO_DRIVERS, &VDriver { id: 18, probe, keeps_config_change: false, interrupt_hook: Some(interrupt) });
register!(CLASSES, &Keyboards);
register!(SOURCES, &Keyboards);
register!(DEVICES, &Keymap);

struct Key {
    pub keyboard: Name,
    pub down: bool,
    pub ch: Option<u32>,
    pub code: u16,
}

struct Keyboards;

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

impl signal::Source for Keyboards {
    fn pending(&self) -> bool {
        let now = time_now();
        STATE.with(|state| state.active().iter().any(|k| k.ready(now)))
    }

    fn take(&self) -> Option<Event> {
        let now = time_now();
        let key = STATE
            .with(|state| state.active().iter_mut().enumerate().find_map(|(index, k)| Some(k.take(now)?.key(index))))?;
        Some(Event {
            int_values: [key.ch.map_or(-1, i64::from), key.code as i64],
            int_value_count: 2,
            ..Event::with_value(if key.down { c"key_down" } else { c"key_up" }, key.keyboard.as_bytes())
        })
    }

    fn deadline(&self) -> u64 {
        let due = |k: &Keyboard| k.repeat.map_or(TIME_NEVER, |r| r.at);
        STATE.with(|state| state.active().iter().map(due).min().unwrap_or(TIME_NEVER))
    }
}

struct Keymap;

impl devfs::Device for Keymap {
    fn count(&self) -> usize {
        1
    }

    fn name(&self, _: usize) -> Option<Name> {
        Some(Name::of(b"keymap"))
    }

    fn open(&self, index: usize, flags: OpenFlags) -> Result<usize> {
        if flags.has(OpenFlags::READ) || !flags.has(OpenFlags::WRITE) {
            return Err(Error::Denied);
        }
        LOADER.with(|loader| {
            if loader.open {
                return Err(Error::Busy);
            }
            *loader = Loader::CLOSED;
            loader.open = true;
            Ok(index)
        })
    }

    fn read(&self, _: usize, _: u64, _: &mut [u8]) -> Result<usize> {
        Err(Error::BadFd)
    }

    fn write(&self, _: usize, _: u64, src: &[u8]) -> Result<usize> {
        LOADER.with(|loader| {
            if loader.failed || src.iter().try_for_each(|&byte| loader.feed(byte)).is_err() {
                loader.failed = true;
                return Err(Error::Invalid);
            }
            Ok(src.len())
        })
    }

    fn close(&self, _: usize) -> Result<()> {
        let loader = LOADER.with(|loader| core::mem::replace(loader, Loader::CLOSED));
        if loader.failed || loader.complete() { Ok(()) } else { Err(Error::Invalid) }
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
const KEY_KPASTERISK: u16 = 55;
const KEY_LEFTALT: u16 = 56;
const KEY_CAPSLOCK: u16 = 58;
const KEY_NUMLOCK: u16 = 69;
const KEY_SCROLLLOCK: u16 = 70;
const KEY_KP7: u16 = 71;
const KEY_KPDOT: u16 = 83;
const KEY_KPENTER: u16 = 96;
const KEY_RIGHTCTRL: u16 = 97;
const KEY_KPSLASH: u16 = 98;
const KEY_RIGHTALT: u16 = 100;

const SHIFT: u8 = 1;
const CTRL: u8 = 2;
const CAPS: u8 = 4;
const LEFT_SHIFT: u8 = 8;
const RIGHT_SHIFT: u8 = 16;
const LEFT_CTRL: u8 = 32;
const RIGHT_CTRL: u8 = 64;
const ALTGR: u8 = 128;

const REPEAT_DELAY_MS: u64 = 250;
const REPEAT_PERIOD_MS: u64 = 33;

const MAX_KEYBOARDS: usize = 4;
const RING_SIZE: usize = 32;
const TRACKED_KEYS: usize = 256;

const KEYS: usize = KEY_KPSLASH as usize + 1;

// Linux US defkeymap, Num Lock on.
const PLAIN: &[u8; KEYS] = b"\0\x1b1234567890-=\x7f\tqwertyuiop[]\r\0asdfghjkl;'`\0\\zxcvbnm,./\0*\0 \0\
    \0\0\0\0\0\0\0\0\0\0\0\x00789-456+1230.\0\0<\0\0\0\0\0\0\0\0\0\r\0/";
const SHIFTED: &[u8; KEYS] = b"\0\x1b!@#$%^&*()_+\x7f\tQWERTYUIOP{}\r\0ASDFGHJKL:\"~\0|ZXCVBNM<>?\0*\0 \0\
    \0\0\0\0\0\0\0\0\0\0\0\x00789-456+1230.\0\0>\0\0\0\0\0\0\0\0\0\r\0/";

// Linux's binary keymap, as `loadkeys -b` writes it: the magic, a flag per table, then each present table.
const BKEYMAP_MAGIC: &[u8; 7] = b"bkeymap";
const BKEYMAP_TABLES: usize = 256;
const BKEYMAP_HEADER: usize = BKEYMAP_MAGIC.len() + BKEYMAP_TABLES;
const BKEYMAP_TABLE: usize = 128 * 2;
const LEVELS: usize = 4; // the tables for no modifier, Shift, AltGr and Shift+AltGr

// Keysym types and keysyms of linux/keyboard.h.
const KT_LATIN: u8 = 0;
const KT_SPEC: u8 = 2;
const KT_PAD: u8 = 3;
const KT_DEAD: u8 = 4;
const KT_META: u8 = 8;
const KT_LETTER: u8 = 11;
const NR_TYPES: u8 = 15;
const K_ENTER: u16 = 0x0201;
const K_ALTGR: u16 = 0x0701;
const UNICODE: u16 = 0xf000;

const PAD_CHARS: &[u8; 21] = b"0123456789+-*/\r,.?()#";
const DEAD_ACCENTS: [u16; 13] = [0x60, 0xb4, 0x5e, 0x7e, 0xa8, 0xb8, 0xaf, 0x2d8, 0x2d9, 0x2da, 0x2dd, 0x2c7, 0x2db];

static STATE: IrqCell<State> = IrqCell::new(State { count: 0, keyboards: [const { Keyboard::EMPTY }; MAX_KEYBOARDS] });
static LAYOUT: KernelCell<Layout> = KernelCell::new(Layout::US);
static LOADER: KernelCell<Loader> = KernelCell::new(Loader::CLOSED);

pub fn count() -> usize {
    STATE.with(|state| state.count)
}

pub fn name(index: usize) -> Option<Name> {
    (index < count()).then(|| input_name(index))
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

fn character(code: u16, modifiers: u8) -> Option<u32> {
    let ch = LAYOUT.with(|layout| layout.character(code, modifiers))?;
    if modifiers & CTRL == 0 || matches!(code, KEY_KPASTERISK | KEY_KP7..=KEY_KPDOT | KEY_KPENTER | KEY_KPSLASH) {
        return Some(ch);
    }
    match char::from_u32(ch)? {
        'a'..='z' | 'A'..='Z' => Some(ch & 0x1f),
        '@' | '2' | ' ' | '`' => Some(0),
        '3' | '[' => Some(0x1b),
        '4' | '\\' => Some(0x1c),
        '5' | ']' => Some(0x1d),
        '6' | '^' => Some(0x1e),
        '7' | '-' | '_' => Some(0x1f),
        '8' | '/' => Some(0x7f),
        '\'' => Some(0x07),
        '\x7f' => Some(0x08),
        '\r' => Some(ch),
        _ => None,
    }
}

fn keysym_character(keysym: u16) -> Result<u16> {
    let [value, kind] = keysym.to_le_bytes();
    let index = value as usize;
    Ok(match kind {
        KT_LATIN | KT_LETTER | KT_META => value as u16,
        KT_SPEC if keysym == K_ENTER => b'\r' as u16,
        KT_PAD => PAD_CHARS.get(index).map_or(0, |&c| c as u16),
        KT_DEAD => DEAD_ACCENTS.get(index).copied().unwrap_or(0),
        NR_TYPES => return Err(Error::Invalid),
        ..NR_TYPES => 0,
        _ => Some(keysym ^ UNICODE).filter(|c| !(0xd800..=0xdfff).contains(c)).ok_or(Error::Invalid)?,
    })
}

#[derive(Clone, Copy)]
struct Layout {
    keys: [[u16; LEVELS]; KEYS],
    caps: [u8; KEYS.div_ceil(8)],
    altgr: bool, // the right Alt key is AltGr, only if the keymap says so, like on Linux
}

impl Layout {
    const EMPTY: Self = Self { keys: [[0; LEVELS]; KEYS], caps: [0; KEYS.div_ceil(8)], altgr: false };

    const US: Self = {
        let mut layout = Self::EMPTY;
        let mut code = 0;
        while code < KEYS {
            layout.keys[code] = [PLAIN[code] as u16, SHIFTED[code] as u16, 0, 0];
            if PLAIN[code].is_ascii_lowercase() {
                layout.caps[code / 8] |= 1 << (code % 8);
            }
            code += 1;
        }
        layout
    };

    fn character(&self, code: u16, modifiers: u8) -> Option<u32> {
        let index = code as usize;
        let [plain, shifted, altgr, shifted_altgr] = *self.keys.get(index)?;
        let caps = modifiers & CAPS != 0 && self.caps.get(index / 8).is_some_and(|bits| bits & (1 << (index % 8)) != 0);
        let shift = modifiers & SHIFT != 0;
        let level3 = if shift { shifted_altgr } else { altgr };
        let ch = if modifiers & ALTGR != 0 && self.altgr && level3 != 0 {
            level3
        } else if shift != caps {
            shifted
        } else {
            plain
        };
        (ch != 0).then_some(ch as u32)
    }
}

struct Loader {
    open: bool,
    failed: bool,
    pos: usize,
    tables: usize,
    kept: usize, // present tables below LEVELS
    levels: u8,  // the level of each kept table, two bits each
    low: u8,     // of the keysym being read
    layout: Layout,
}

impl Loader {
    const CLOSED: Self =
        Self { open: false, failed: false, pos: 0, tables: 0, kept: 0, levels: 0, low: 0, layout: Layout::EMPTY };

    fn complete(&self) -> bool {
        self.pos == BKEYMAP_HEADER + self.tables * BKEYMAP_TABLE
    }

    fn feed(&mut self, byte: u8) -> Result<()> {
        self.parse(byte)?;
        if self.complete() {
            LAYOUT.with(|layout| *layout = self.layout);
        }
        Ok(())
    }

    fn parse(&mut self, byte: u8) -> Result<()> {
        let pos = self.pos;
        self.pos += 1;
        if let Some(&expected) = BKEYMAP_MAGIC.get(pos) {
            return if byte == expected { Ok(()) } else { Err(Error::Invalid) };
        }
        if pos < BKEYMAP_HEADER {
            if byte > 1 {
                return Err(Error::Invalid);
            }
            let table = pos - BKEYMAP_MAGIC.len();
            if byte == 1 && table < LEVELS {
                self.levels |= (table as u8) << (2 * self.kept);
                self.kept += 1;
            }
            self.tables += byte as usize;
            return Ok(());
        }
        let entry = pos - BKEYMAP_HEADER;
        let (table, code) = (entry / BKEYMAP_TABLE, entry % BKEYMAP_TABLE / 2);
        if table >= self.tables {
            return Err(Error::Invalid);
        }
        if entry.is_multiple_of(2) {
            self.low = byte;
            return Ok(());
        }
        if table >= self.kept {
            return Ok(());
        }
        let level = (self.levels >> (2 * table)) as usize & 3;
        let keysym = u16::from_le_bytes([self.low, byte]);
        self.layout.altgr |= level == 0 && code == KEY_RIGHTALT as usize && keysym == K_ALTGR;
        let Some(ch) = self.layout.keys.get_mut(code).and_then(|key| key.get_mut(level)) else { return Ok(()) };
        *ch = keysym_character(keysym)?;
        if level == 0
            && keysym >> 8 == KT_LETTER as u16
            && let Some(bits) = self.layout.caps.get_mut(code / 8)
        {
            *bits |= 1 << (code % 8);
        }
        Ok(())
    }
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
            KEY_RIGHTALT => ALTGR,
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
        shift | ctrl | (self.modifiers & (CAPS | ALTGR))
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
