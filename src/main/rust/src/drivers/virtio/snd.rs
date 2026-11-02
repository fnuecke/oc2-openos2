// SPDX-License-Identifier: MIT
//! virtio-snd, as OSS-style /dev/dsp (U8), /dev/dspW (S16_LE) and /dev/audio (mu-law), mono at 11025 Hz.

use super::{Buf, Device, Driver, Virtq};
use crate::arch::{KernelCell, time_now};
use crate::fs::devfs::{self, Poll};
use crate::fs::{Error, Name, OpenFlags, Result};
use crate::mem::HeapBuf;
use crate::time::{app_wait, frequency, wait};

pub const DRIVER: Driver = Driver { id: 25, probe, keeps_config_change: false };

pub struct Dsp;

pub fn signal_ready() -> bool {
    STATE.with(State::signal_due)
}

pub fn take_signal() -> Option<Name> {
    STATE.with(|state| {
        let stream = state.signal_due().then_some(state.stream.as_mut()).flatten()?;
        stream.blocked = false;
        Some(Name::of(stream.name))
    })
}

impl devfs::Device for Dsp {
    fn count(&self) -> usize {
        if STATE.with(|state| state.device.is_some()) { FORMATS.len() } else { 0 }
    }

    fn name(&self, index: usize) -> Option<Name> {
        FORMATS.get(index).filter(|_| self.count() > 0).map(|format| Name::of(format.name))
    }

    fn open(&self, index: usize, flags: OpenFlags) -> Result<usize> {
        let format = FORMATS.get(index).ok_or(Error::NotFound)?;
        if !flags.has(OpenFlags::WRITE) {
            return Err(Error::Invalid);
        }
        STATE.with(|state| state.open(format, flags))?;
        Ok(index)
    }

    fn read(&self, _: usize, _: u64, _: &mut [u8]) -> Result<usize> {
        let readable = STATE.with(|state| state.stream.as_ref().is_some_and(|s| s.readable));
        Err(if readable { Error::NoAddress } else { Error::BadFd })
    }

    fn write(&self, _: usize, _: u64, src: &[u8]) -> Result<usize> {
        let nonblocking = STATE.with(|state| state.stream.as_ref().map(|s| s.nonblocking)).ok_or(Error::BadFd)?;
        let mut done = 0;
        loop {
            match STATE.with(|state| state.queue(src.get(done..).unwrap_or_default())) {
                Ok(count) => done += count,
                Err(_) if done > 0 => return Ok(done),
                Err(e) => return Err(e),
            }
            if done == src.len() {
                return Ok(done);
            }
            let deadline = time_now() + PERIODS as u64 * period_ticks() + frequency();
            let error = if nonblocking {
                STATE.with(State::block);
                Error::Again
            } else if app_wait(|| STATE.with(State::writable), deadline) {
                continue;
            } else if time_now() >= deadline {
                Error::Io
            } else {
                Error::Interrupted
            };
            return if done > 0 { Ok(done) } else { Err(error) };
        }
    }

    fn poll(&self, _: usize) -> Poll {
        let writable = STATE.with(State::writable);
        if !writable {
            STATE.with(State::block);
        }
        Poll { readable: false, writable }
    }

    fn close(&self, _: usize) -> Result<()> {
        let flushed = STATE.with(State::flush);
        if let Ok(deadline) = flushed {
            app_wait(|| STATE.with(State::drained), deadline);
        }
        let released = STATE.with(State::release);
        flushed.and(released)
    }
}

const CONFIG_STREAMS: u32 = 4;

const CONTROL_QUEUE: u16 = 0;
const TX_QUEUE: u16 = 2;

const R_PCM_SET_PARAMS: u32 = 0x0101;
const R_PCM_PREPARE: u32 = 0x0102;
const R_PCM_RELEASE: u32 = 0x0103;
const R_PCM_START: u32 = 0x0104;
const R_PCM_STOP: u32 = 0x0105;
const S_OK: u32 = 0x8000;

const FMT_MU_LAW: u8 = 1;
const FMT_U8: u8 = 4;
const FMT_S16: u8 = 5;
const RATE_11025: u8 = 2;
const SAMPLE_RATE: u64 = 11025;

// 46 ms per period, 186 ms in the buffer, 4144 bytes for S16.
const PERIOD_FRAMES: usize = 512;
const PERIODS: usize = 4;
const XFER_SIZE: usize = 4;
const STATUS_SIZE: usize = 8;
const HEADER_SIZE: u32 = 8;
const SET_PARAMS_SIZE: u32 = 24;

const FORMATS: [Format; 3] = [
    Format { name: b"dsp", code: FMT_U8, sample_bytes: 1, silence: 0x80 },
    Format { name: b"dspW", code: FMT_S16, sample_bytes: 2, silence: 0 },
    Format { name: b"audio", code: FMT_MU_LAW, sample_bytes: 1, silence: 0xFF },
];

static STATE: KernelCell<State> = KernelCell::new(State {
    device: None,
    control: Virtq::EMPTY,
    tx: Virtq::EMPTY,
    request: Request::EMPTY,
    response: 0,
    stream: None,
});

fn probe(device: Device) -> bool {
    if device.begin(0).is_none() || device.config32(CONFIG_STREAMS) == 0 {
        return false;
    }
    STATE.with(|state| {
        if state.device.is_some()
            || state.control.init(device, CONTROL_QUEUE, 2).is_err()
            || state.tx.init(device, TX_QUEUE, 2 * PERIODS as u16).is_err()
        {
            return false;
        }
        device.ready();
        state.device = Some(device);
        true
    })
}

fn period_ticks() -> u64 {
    frequency() * PERIOD_FRAMES as u64 / SAMPLE_RATE
}

struct Format {
    name: &'static [u8],
    code: u8,
    sample_bytes: usize,
    silence: u8,
}

#[repr(C)]
struct Request {
    code: u32,
    stream: u32,
    buffer_bytes: u32,
    period_bytes: u32,
    features: u32,
    channels: u8,
    format: u8,
    rate: u8,
    padding: u8,
}

impl Request {
    const EMPTY: Self = Self {
        code: 0,
        stream: 0,
        buffer_bytes: 0,
        period_bytes: 0,
        features: 0,
        channels: 0,
        format: 0,
        rate: 0,
        padding: 0,
    };
}

struct Stream {
    name: &'static [u8],
    slots: HeapBuf,
    period: usize,
    silence: u8,
    nonblocking: bool,
    readable: bool,
    blocked: bool,
    started: bool,
    head: usize,
    queued: usize,
    fill: usize,
}

impl Stream {
    fn slot(&mut self, index: usize) -> Option<&mut [u8]> {
        let size = XFER_SIZE + self.period + STATUS_SIZE;
        self.slots.get_mut(index * size..(index + 1) * size)
    }

    fn submit(&mut self, tx: &mut Virtq, index: usize) -> Result<()> {
        let period = self.period;
        let slot = self.slot(index).ok_or(Error::Io)?;
        let (xfer, status) = (slot.as_mut_ptr(), slot.as_mut_ptr().wrapping_add(XFER_SIZE + period));
        let bufs = [
            Buf { addr: xfer.cast(), len: (XFER_SIZE + period) as u32 },
            Buf { addr: status.cast(), len: STATUS_SIZE as u32 },
        ];
        tx.add(&bufs, 1).ok_or(Error::Io)?;
        tx.notify();
        self.queued += 1;
        self.fill = 0;
        Ok(())
    }
}

struct State {
    device: Option<Device>,
    control: Virtq,
    tx: Virtq,
    request: Request,
    response: u32,
    stream: Option<Stream>,
}

impl State {
    fn open(&mut self, format: &Format, flags: OpenFlags) -> Result<()> {
        if self.device.is_none() {
            return Err(Error::Io);
        }
        if self.stream.is_some() {
            return Err(Error::Busy);
        }
        let period = PERIOD_FRAMES * format.sample_bytes;
        let slots = HeapBuf::zeroed(PERIODS * (XFER_SIZE + period + STATUS_SIZE)).ok_or(Error::NoSpace)?;
        self.request = Request {
            code: R_PCM_SET_PARAMS,
            buffer_bytes: (PERIODS * period) as u32,
            period_bytes: period as u32,
            channels: 1,
            format: format.code,
            rate: RATE_11025,
            ..Request::EMPTY
        };
        self.control(SET_PARAMS_SIZE)?;
        self.command(R_PCM_PREPARE)?;
        self.stream = Some(Stream {
            name: format.name,
            slots,
            period,
            silence: format.silence,
            nonblocking: flags.has(OpenFlags::NONBLOCK),
            readable: flags.has(OpenFlags::READ),
            blocked: false,
            started: false,
            head: 0,
            queued: 0,
            fill: 0,
        });
        Ok(())
    }

    fn queue(&mut self, src: &[u8]) -> Result<usize> {
        self.check()?;
        let State { tx, stream, .. } = self;
        let stream = stream.as_mut().ok_or(Error::BadFd)?;
        let mut done = 0;
        let mut result = Ok(());
        while done < src.len() && stream.queued < PERIODS && result.is_ok() {
            let (index, at) = ((stream.head + stream.queued) % PERIODS, XFER_SIZE + stream.fill);
            let count = (stream.period - stream.fill).min(src.len() - done);
            let data = src.get(done..done + count);
            let Some((dst, data)) = stream.slot(index).and_then(|slot| slot.get_mut(at..at + count)).zip(data) else {
                result = Err(Error::Io);
                break;
            };
            dst.copy_from_slice(data);
            stream.fill += count;
            done += count;
            if stream.fill == stream.period {
                result = stream.submit(tx, index);
            }
        }
        match (done, result.and_then(|_| self.start())) {
            (0, Err(e)) => Err(e),
            _ => Ok(done),
        }
    }

    fn flush(&mut self) -> Result<u64> {
        self.check()?;
        let State { tx, stream, .. } = self;
        let stream = stream.as_mut().ok_or(Error::BadFd)?;
        if stream.fill > 0 {
            let (index, at, silence) =
                ((stream.head + stream.queued) % PERIODS, XFER_SIZE + stream.fill, stream.silence);
            let end = XFER_SIZE + stream.period;
            if let Some(rest) = stream.slot(index).and_then(|slot| slot.get_mut(at..end)) {
                rest.fill(silence);
            }
            stream.submit(tx, index)?;
        }
        let deadline = time_now() + stream.queued as u64 * period_ticks() + frequency();
        self.start()?;
        Ok(deadline)
    }

    fn release(&mut self) -> Result<()> {
        let started = self.stream.as_ref().ok_or(Error::BadFd)?.started;
        let stopped = if started { self.command(R_PCM_STOP) } else { Ok(()) };
        let released = self.command(R_PCM_RELEASE);
        self.reclaim();
        if self.stream.as_ref().is_some_and(|s| s.queued > 0)
            && let Some(device) = self.device.take()
        {
            device.reset();
        }
        self.stream = None;
        stopped.and(released)
    }

    fn start(&mut self) -> Result<()> {
        if !self.stream.as_ref().is_some_and(|s| !s.started && s.queued > 0) {
            return Ok(());
        }
        self.command(R_PCM_START)?;
        if let Some(stream) = &mut self.stream {
            stream.started = true;
        }
        Ok(())
    }

    fn check(&mut self) -> Result<()> {
        self.reclaim();
        match self.device {
            Some(device) if !device.failed() => Ok(()),
            device => {
                device.inspect(|d| d.reset());
                self.device = None;
                Err(Error::Io)
            }
        }
    }

    fn reclaim(&mut self) {
        while self.tx.next_used().is_some() {
            if let Some(stream) = &mut self.stream {
                stream.queued = stream.queued.saturating_sub(1);
                stream.head = (stream.head + 1) % PERIODS;
            }
        }
    }

    fn signal_due(&mut self) -> bool {
        self.stream.as_ref().is_some_and(|s| s.blocked) && self.writable()
    }

    fn block(&mut self) {
        if let Some(stream) = &mut self.stream {
            stream.blocked = true;
        }
    }

    fn writable(&mut self) -> bool {
        self.reclaim();
        self.device.is_none_or(Device::failed) || self.stream.as_ref().is_some_and(|s| s.queued < PERIODS)
    }

    fn drained(&mut self) -> bool {
        self.reclaim();
        self.device.is_none_or(Device::failed) || self.stream.as_ref().is_none_or(|s| s.queued == 0)
    }

    fn command(&mut self, code: u32) -> Result<()> {
        self.request = Request { code, ..Request::EMPTY };
        self.control(HEADER_SIZE)
    }

    fn control(&mut self, len: u32) -> Result<()> {
        let device = self.device.ok_or(Error::Io)?;
        if device.failed() {
            return Err(Error::Io);
        }
        self.response = 0;
        let bufs = [
            Buf { addr: (&raw const self.request).cast(), len },
            Buf { addr: (&raw const self.response).cast(), len: size_of::<u32>() as u32 },
        ];
        self.control.add(&bufs, 1).ok_or(Error::Io)?;
        self.control.notify();
        let control = &self.control;
        wait(|| control.has_used() || device.failed(), time_now() + frequency());
        if self.control.next_used().is_none() {
            device.reset();
            self.device = None;
            return Err(Error::Io);
        }
        match unsafe { (&raw const self.response).read_volatile() } {
            S_OK => Ok(()),
            _ => Err(Error::Io),
        }
    }
}
