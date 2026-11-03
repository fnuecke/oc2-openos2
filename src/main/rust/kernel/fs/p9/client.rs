// SPDX-License-Identifier: MIT

use crate::arch::time_now;
use crate::bytes::{le16, le64};
use crate::drivers::virtio::{Buf, Device, Virtq};
use crate::fs::{Error, Kind, Listing, MNT, MountText, OpenFlags, Result, Stat, is_valid_name, next_name};
use crate::mem::HeapBox;
use crate::time::{frequency, wait};

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Fid(u32);

pub struct Mount {
    pub path: MountText,
    pub read_only: bool,
    device: Device,
    dead: bool,
    queue: Virtq,
    fids: u32,
    tx: [u8; TX_SIZE],
    rx: [u8; RX_SIZE],
}

impl Mount {
    pub fn tag(&self) -> &[u8] {
        &self.path.as_bytes()[MNT.len()..]
    }

    pub fn getattr(&mut self, fid: Fid) -> Result<Stat> {
        let end = self.begin(TGETATTR).fid(fid).u64(GETATTR_BASIC).end();
        self.request(end)?;
        let kind = if self.reply(HEADER + 21) & S_IFMT == S_IFDIR { Kind::Directory } else { Kind::File };
        Ok(Stat { size: self.reply64(HEADER + 49), mtime: self.reply(HEADER + 89), kind })
    }

    pub fn info(&mut self) -> Result<(u64, u64)> {
        let end = self.begin(TSTATFS).fid(ROOT_FID).end();
        self.request(end)?;
        let block_size = self.reply(HEADER + 4) as u64;
        Ok((self.reply64(HEADER + 8).wrapping_mul(block_size), self.reply64(HEADER + 16).wrapping_mul(block_size)))
    }

    pub fn stat(&mut self, path: &[u8]) -> Result<Stat> {
        let (fid, _) = self.walk_path(path, false)?;
        let result = self.getattr(fid);
        self.clunk(fid);
        result
    }

    pub fn list(&mut self, path: &[u8], buffer: &mut [u8; DIR_BUFFER], out: &mut Listing) -> Result<()> {
        let (fid, _) = self.walk_path(path, false)?;
        let end = self.begin(TLOPEN).fid(fid).u32(L_O_RDONLY | L_O_DIRECTORY).end();
        let mut result = self.request(end);
        let mut offset = 0;
        while result.is_ok() {
            let end = self.begin(TREADDIR).fid(fid).u64(offset).u32(DIR_BUFFER as u32).end();
            result = self.transact(end, &[], IO_REPLY, (buffer.as_mut_ptr(), DIR_BUFFER));
            let count = if result.is_ok() { self.reply(HEADER) as usize } else { 0 };
            if count == 0 {
                break;
            }
            let entries = unsafe { core::slice::from_raw_parts(buffer.as_ptr(), count.min(DIR_BUFFER)) };
            let mut at = 0;
            while result.is_ok() {
                let e = &entries[at..];
                let (Some(next), Some(len)) = (le64(e, 13), le16(e, 22)) else { break };
                let Some(name) = e.get(24..24 + len as usize) else { break };
                offset = next;
                if name != b"." && name != b".." {
                    result = out.add(name, if e[21] == DT_DIR { Kind::Directory } else { Kind::File });
                }
                at += 24 + name.len();
            }
        }
        self.clunk(fid);
        result
    }

    pub fn open(&mut self, path: &[u8], flags: OpenFlags) -> Result<Fid> {
        let mut mode = if flags.has(OpenFlags::READ) && flags.has(OpenFlags::WRITE) {
            L_O_RDWR
        } else if flags.has(OpenFlags::WRITE) {
            L_O_WRONLY
        } else {
            L_O_RDONLY
        };
        if flags.has(OpenFlags::TRUNCATE) && flags.has(OpenFlags::WRITE) {
            mode |= L_O_TRUNC;
        }
        let (fid, result) = match self.walk_path(path, false) {
            Err(Error::NotFound) if flags.has(OpenFlags::CREATE) => {
                let (fid, name) = self.walk_path(path, true)?;
                let result = self.writable().and_then(|_| {
                    let end = self.begin(TLCREATE).fid(fid).string(name).u32(mode).u32(0o644).u32(0).end();
                    self.request(end)
                });
                (fid, result)
            }
            Err(e) => return Err(e),
            Ok((fid, _)) => {
                let result = self
                    .getattr(fid)
                    .and_then(|st| if st.kind == Kind::Directory { Err(Error::IsDir) } else { Ok(()) })
                    .and_then(|_| if flags.has(OpenFlags::MODIFY) { self.writable() } else { Ok(()) });
                let result = result.and_then(|_| {
                    let end = self.begin(TLOPEN).fid(fid).u32(mode).end();
                    self.request(end)
                });
                (fid, result)
            }
        };
        if let Err(e) = result {
            self.clunk(fid);
            return Err(e);
        }
        Ok(fid)
    }

    pub fn mkdir(&mut self, path: &[u8]) -> Result<()> {
        self.at_parent(path, |m, fid, name| m.begin(TMKDIR).fid(fid).string(name).u32(0o755).u32(0).end())
    }

    pub fn remove(&mut self, path: &[u8]) -> Result<()> {
        self.at_parent(path, |m, fid, name| m.begin(TUNLINKAT).fid(fid).string(name).u32(0).end())
    }

    pub fn rename(&mut self, from: &[u8], to: &[u8]) -> Result<()> {
        self.writable()?;
        let (from_fid, from_name) = self.walk_path(from, true)?;
        let (to_fid, to_name) = match self.walk_path(to, true) {
            Ok(walked) => walked,
            Err(e) => {
                self.clunk(from_fid);
                return Err(e);
            }
        };
        let end = self.begin(TRENAMEAT).fid(from_fid).string(from_name).fid(to_fid).string(to_name).end();
        let result = self.request(end);
        self.clunk(from_fid);
        self.clunk(to_fid);
        result
    }

    pub fn read(&mut self, fid: Fid, pos: &mut u64, dst: &mut [u8]) -> Result<usize> {
        let mut done = 0;
        while done < dst.len() {
            let want = (dst.len() - done).min(CHUNK);
            let end = self.begin(TREAD).fid(fid).u64(*pos).u32(want as u32).end();
            if let Err(e) = self.transact(end, &[], IO_REPLY, (dst[done..].as_mut_ptr(), want)) {
                return if done > 0 { Ok(done) } else { Err(e) };
            }
            let mut got = self.reply(HEADER) as usize;
            if got > want {
                got = 0;
            }
            done += got;
            *pos += got as u64;
            if got < want {
                break;
            }
        }
        Ok(done)
    }

    pub fn write(&mut self, fid: Fid, pos: &mut u64, append: bool, src: &[u8]) -> Result<usize> {
        if append {
            *pos = self.getattr(fid)?.size;
        }
        let mut done = 0;
        while done < src.len() {
            let want = (src.len() - done).min(CHUNK);
            let end = self.begin(TWRITE).fid(fid).u64(*pos).u32(want as u32).end();
            if let Err(e) = self.transact(end, &src[done..done + want], IO_REPLY, (core::ptr::null_mut(), 0)) {
                return if done > 0 { Ok(done) } else { Err(e) };
            }
            let wrote = self.reply(HEADER) as usize;
            done += wrote;
            *pos += wrote as u64;
            if wrote == 0 {
                break;
            }
        }
        Ok(done)
    }

    pub fn clunk(&mut self, fid: Fid) {
        let end = self.begin(TCLUNK).fid(fid).end();
        let _ = self.transact(end, &[], HEADER as u32, (core::ptr::null_mut(), 0));
        self.fids &= !(1 << fid.0);
    }

    fn begin(&mut self, kind: u8) -> Message<'_> {
        self.tx[4] = kind;
        let tag = if kind == TVERSION { NOTAG } else { TAG };
        self.tx[5..7].copy_from_slice(&tag.to_le_bytes());
        Message { tx: &mut self.tx, at: HEADER }
    }

    fn reply(&self, at: usize) -> u32 {
        let p = self.rx.as_ptr();
        u32::from_le_bytes(core::array::from_fn(|i| unsafe { p.add(at + i).read_volatile() }))
    }

    fn reply64(&self, at: usize) -> u64 {
        self.reply(at) as u64 | (self.reply(at + 4) as u64) << 32
    }

    fn reply_bytes(&self) -> [u8; RX_SIZE] {
        let p = self.rx.as_ptr();
        core::array::from_fn(|i| unsafe { p.add(i).read_volatile() })
    }

    fn transact(&mut self, end: usize, out: &[u8], reply: u32, input: (*mut u8, usize)) -> Result<()> {
        if self.dead {
            return Err(Error::Io);
        }
        let length = (end + out.len()) as u32;
        self.tx[..4].copy_from_slice(&length.to_le_bytes());
        let mut bufs = [const { Buf { addr: core::ptr::null(), len: 0 } }; 4];
        let mut count = 0;
        bufs[count] = Buf { addr: self.tx.as_ptr().cast(), len: end as u32 };
        count += 1;
        if !out.is_empty() {
            bufs[count] = Buf { addr: out.as_ptr().cast(), len: out.len() as u32 };
            count += 1;
        }
        let out_count = count;
        bufs[count] = Buf { addr: self.rx.as_ptr().cast(), len: reply };
        count += 1;
        if input.1 > 0 {
            bufs[count] = Buf { addr: input.0.cast(), len: input.1 as u32 };
            count += 1;
        }
        let kind = self.tx[4];
        self.rx[4] = 0;
        self.queue.add(&bufs[..count], out_count).ok_or(Error::Io)?;
        self.queue.notify();

        let deadline = time_now() + TIMEOUT_SECONDS * frequency();
        while !self.queue.has_used() {
            if time_now() >= deadline {
                self.device.reset();
                self.dead = true;
                return Err(Error::Io);
            }
            let queue = &self.queue;
            wait(|| queue.has_used(), deadline);
        }
        if self.queue.next_used().is_none() {
            self.device.reset();
            self.dead = true;
            return Err(Error::Io);
        }

        let replied = unsafe { self.rx.as_ptr().add(4).read_volatile() };
        if replied == TLERROR + 1 {
            let errno = self.reply(HEADER);
            return Err(match Error::from_errno(errno) {
                Some(Error::Again | Error::Changed | Error::NoMedium | Error::Unformatted) | None => Error::Io,
                Some(e) => e,
            });
        }
        if replied == kind + 1 { Ok(()) } else { Err(Error::Io) }
    }

    fn request(&mut self, end: usize) -> Result<()> {
        self.transact(end, &[], RX_SIZE as u32, (core::ptr::null_mut(), 0))
    }

    fn alloc_fid(&mut self) -> Result<Fid> {
        let fid = (1..32).find(|fid| self.fids & 1 << fid == 0).ok_or(Error::TooMany)?;
        self.fids |= 1 << fid;
        Ok(Fid(fid))
    }

    fn walk(&mut self, newfid: Fid, names: &[&[u8]]) -> Result<()> {
        let mut message = self.begin(TWALK).fid(ROOT_FID).fid(newfid).u16(names.len() as u16);
        for name in names {
            if message.at + 2 + name.len() > TX_SIZE {
                return Err(Error::NameTooLong);
            }
            message = message.string(name);
        }
        let end = message.end();
        self.request(end)?;
        let walked = self.reply(HEADER) & 0xFFFF;
        if walked as usize == names.len() { Ok(()) } else { Err(Error::NotFound) }
    }

    fn walk_path<'a>(&mut self, path: &'a [u8], parent: bool) -> Result<(Fid, &'a [u8])> {
        let names = Names::split(path)?;
        let mut names = names.as_slice();
        let mut last: &[u8] = &[];
        if parent {
            let (name, rest) = names.split_last().ok_or(Error::Invalid)?;
            last = name;
            names = rest;
        }
        let fid = self.alloc_fid()?;
        self.walk(fid, names).inspect_err(|_| self.fids &= !(1 << fid.0))?;
        Ok((fid, last))
    }

    fn writable(&self) -> Result<()> {
        if self.read_only { Err(Error::ReadOnly) } else { Ok(()) }
    }

    fn at_parent(&mut self, path: &[u8], message: impl FnOnce(&mut Self, Fid, &[u8]) -> usize) -> Result<()> {
        self.writable()?;
        let (fid, name) = self.walk_path(path, true)?;
        let end = message(self, fid, name);
        let result = self.request(end);
        self.clunk(fid);
        result
    }
}

pub const DIR_BUFFER: usize = 1024;

const F_MOUNT_TAG: u32 = 1 << 0;

const TLERROR: u8 = 6;
const TSTATFS: u8 = 8;
const TLOPEN: u8 = 12;
const TLCREATE: u8 = 14;
const TGETATTR: u8 = 24;
const TREADDIR: u8 = 40;
const TMKDIR: u8 = 72;
const TRENAMEAT: u8 = 74;
const TUNLINKAT: u8 = 76;
const TVERSION: u8 = 100;
const TATTACH: u8 = 104;
const TWALK: u8 = 110;
const TREAD: u8 = 116;
const TWRITE: u8 = 118;
const TCLUNK: u8 = 120;

const NOTAG: u16 = 0xFFFF;
const NOFID: Fid = Fid(u32::MAX);
const TAG: u16 = 1;
const ROOT_FID: Fid = Fid(0);

const L_O_RDONLY: u32 = 0;
const L_O_WRONLY: u32 = 1;
const L_O_RDWR: u32 = 2;
const L_O_TRUNC: u32 = 0x200;
const L_O_DIRECTORY: u32 = 0x10000;
const GETATTR_BASIC: u64 = 0x7FF;
const DT_DIR: u8 = 4;
const S_IFMT: u32 = 0o170000;
const S_IFDIR: u32 = 0o040000;

const HEADER: usize = 7;
const IO_REPLY: u32 = 11;
const MSIZE: u32 = 8192;
const CHUNK: usize = MSIZE as usize - 64;
const TX_SIZE: usize = 536; // TRENAMEAT with two names of 255 bytes is 529
const RX_SIZE: usize = 256;
const QUEUE_SIZE: u16 = 4;
const MAX_WALK: usize = 16;
const TIMEOUT_SECONDS: u64 = 10;

pub fn attach(device: Device) -> Option<*mut Mount> {
    let features = device.begin(F_MOUNT_TAG)?;
    let tag_length = device.config8(0) as usize | (device.config8(1) as usize) << 8;
    let mut tag = [0u8; 32];
    let valid = features & F_MOUNT_TAG != 0 && tag_length != 0 && tag_length < tag.len() && {
        for (i, c) in tag[..tag_length].iter_mut().enumerate() {
            *c = device.config8(2 + i as u32);
        }
        is_valid_name(&tag[..tag_length])
    };
    if !valid {
        return None;
    }

    let mut mount = unsafe { HeapBox::<Mount>::zeroed() }?;
    mount.queue.init(device, 0, QUEUE_SIZE).ok()?;
    mount.device = device;
    mount.path = MountText::of(MNT);
    mount.path.push(&tag[..tag_length]);
    device.ready();

    let end = mount.begin(TVERSION).u32(MSIZE).string(b"9P2000.L").end();
    let mut result = mount.request(end);
    if result.is_ok() {
        let r = mount.reply_bytes();
        if mount.reply(HEADER) < MSIZE || &r[HEADER + 4..HEADER + 14] != b"\x08\x009P2000.L" {
            result = Err(Error::Invalid);
        }
    }
    if result.is_ok() {
        let end = mount.begin(TATTACH).fid(ROOT_FID).fid(NOFID).string(b"").string(b"").u32(0).end();
        result = mount.request(end);
    }
    if result.is_err() {
        device.reset(); // before the mount's buffers are freed
        return None;
    }
    mount.fids = 1 << ROOT_FID.0;
    if mount.getattr(ROOT_FID).is_ok() {
        mount.read_only = mount.reply(HEADER + 21) & 0o222 == 0;
    }
    Some(HeapBox::forget(mount))
}

struct Message<'a> {
    tx: &'a mut [u8; TX_SIZE],
    at: usize,
}

impl Message<'_> {
    fn bytes(mut self, data: &[u8]) -> Self {
        self.tx[self.at..self.at + data.len()].copy_from_slice(data);
        self.at += data.len();
        self
    }

    fn u16(self, value: u16) -> Self {
        self.bytes(&value.to_le_bytes())
    }

    fn u32(self, value: u32) -> Self {
        self.bytes(&value.to_le_bytes())
    }

    fn fid(self, fid: Fid) -> Self {
        self.u32(fid.0)
    }

    fn u64(self, value: u64) -> Self {
        self.bytes(&value.to_le_bytes())
    }

    fn string(self, s: &[u8]) -> Self {
        self.u16(s.len() as u16).bytes(s)
    }

    fn end(self) -> usize {
        self.at
    }
}

struct Names<'a> {
    names: [&'a [u8]; MAX_WALK],
    count: usize,
}

impl<'a> Names<'a> {
    fn split(mut path: &'a [u8]) -> Result<Self> {
        let mut names = Self { names: [&[]; MAX_WALK], count: 0 };
        while let Some((name, rest)) = next_name(path) {
            path = rest;
            if name == b"." {
                continue;
            }
            if names.count == MAX_WALK || name.len() > 255 {
                return Err(Error::NameTooLong);
            }
            names.names[names.count] = name;
            names.count += 1;
        }
        Ok(names)
    }

    fn as_slice(&self) -> &[&'a [u8]] {
        &self.names[..self.count]
    }
}
