// SPDX-License-Identifier: MIT

mod disk;
mod memory;

pub use disk::disk;

use super::vfs::{BackendFd, FileSystem, MountId};
use super::{Error, Kind, Listing, Name, OpenFlags, Result, Stat, trim_slashes};
use crate::arch::KernelCell;
use crate::drivers::fb::Framebuffers;
use crate::drivers::uart::Ttys;
use crate::drivers::virtio::console::Ports;
use crate::drivers::virtio::snd::Dsp;
use core::ffi::c_int;
use disk::Disks;
use memory::Memory;

pub const MAX_FILES: usize = 8;

pub struct Poll {
    pub readable: bool,
    pub writable: bool,
}

pub trait Device: Sync {
    fn count(&self) -> usize;

    fn name(&self, index: usize) -> Option<Name>;

    fn find(&self, name: &[u8]) -> Option<usize> {
        (0..self.count()).find(|&i| self.name(i).is_some_and(|n| n.as_bytes() == name))
    }

    fn open(&self, index: usize, _flags: OpenFlags) -> Result<usize> {
        Ok(index)
    }

    fn read(&self, handle: usize, pos: u64, dst: &mut [u8]) -> Result<usize>;

    fn write(&self, handle: usize, pos: u64, src: &[u8]) -> Result<usize>;

    fn seekable(&self) -> bool {
        false
    }

    fn size(&self, _handle: usize) -> Result<u64> {
        Ok(0)
    }

    fn poll(&self, _handle: usize) -> Poll {
        Poll { readable: true, writable: true }
    }

    fn nonblocking(&self) -> bool {
        false
    }

    fn close(&self, _handle: usize) -> Result<()> {
        Ok(())
    }
}

pub struct Devfs;

impl FileSystem for Devfs {
    fn stat(&self, _: MountId, path: &[u8]) -> Result<Stat> {
        let kind = if lookup(path)?.is_none() { Kind::Directory } else { Kind::Device };
        Ok(Stat { kind, ..Stat::default() })
    }

    fn list(&self, _: MountId, path: &[u8], out: &mut Listing) -> Result<()> {
        if lookup(path)?.is_some() {
            return Err(Error::NotDir);
        }
        for device in DEVICES {
            for name in (0..device.count()).filter_map(|i| device.name(i)) {
                let _ = out.add(name.as_bytes(), Kind::Device);
            }
        }
        Ok(())
    }

    fn open(&self, _: MountId, path: &[u8], flags: OpenFlags) -> Result<BackendFd> {
        let (kind, index) = lookup(path)?.ok_or(Error::IsDir)?;
        let device = DEVICES[kind];
        if device.nonblocking() && !flags.has(OpenFlags::NONBLOCK) {
            return Err(Error::Invalid);
        }
        add_file(kind, || device.open(index, flags))
    }

    fn read(&self, fd: BackendFd, dst: &mut [u8]) -> Result<usize> {
        let file = with_file(fd, |f| *f)?;
        let count = file.device().read(file.handle, file.pos, dst)?;
        file.advance(fd, count)
    }

    fn write(&self, fd: BackendFd, src: &[u8]) -> Result<usize> {
        let file = with_file(fd, |f| *f)?;
        let count = file.device().write(file.handle, file.pos, src)?;
        file.advance(fd, count)
    }

    fn seek(&self, fd: BackendFd, offset: i64, whence: c_int) -> Result<usize> {
        let file = with_file(fd, |f| *f)?;
        let device = file.device();
        if !device.seekable() {
            return Ok(0);
        }
        let pos = super::seek(file.pos, offset, whence, || device.size(file.handle))?;
        with_file(fd, |f| f.pos = pos)?;
        Ok(pos as usize)
    }

    fn poll(&self, fd: BackendFd) -> Result<Poll> {
        let file = with_file(fd, |f| *f)?;
        Ok(file.device().poll(file.handle))
    }

    fn close(&self, fd: BackendFd) -> Result<()> {
        let last = FILES.with(|files| {
            let slot = files.get_mut(fd.0).ok_or(Error::Invalid)?;
            let file = slot.as_mut().ok_or(Error::Invalid)?;
            file.opens -= 1;
            Ok(if file.opens == 0 { slot.take() } else { None })
        })?;
        last.map_or(Ok(()), |file| file.device().close(file.handle))
    }
}

const DEVICES: &[&dyn Device] = &[&Memory, &Disks, &Ttys, &Ports, &Framebuffers, &Dsp];
static FILES: KernelCell<[Option<File>; MAX_FILES]> = KernelCell::new([None; MAX_FILES]);

fn lookup(path: &[u8]) -> Result<Option<(usize, usize)>> {
    let name = trim_slashes(path);
    if name.is_empty() {
        return Ok(None);
    }
    let found = DEVICES.iter().enumerate().find_map(|(kind, device)| device.find(name).map(|index| (kind, index)));
    found.map(Some).ok_or(Error::NotFound)
}

fn add_file(kind: usize, open: impl FnOnce() -> Result<usize>) -> Result<BackendFd> {
    let device = DEVICES[kind];
    let shared = device.nonblocking();
    if !shared && FILES.with(|files| files.iter().all(Option::is_some)) {
        return Err(Error::TooMany);
    }
    let handle = open()?;
    let fd = FILES.with(|files| {
        let same = |f: &File| shared && f.kind == kind && f.handle == handle;
        if let Some(fd) = files.iter().position(|f| f.as_ref().is_some_and(same)) {
            files[fd].as_mut()?.opens += 1;
            return Some(BackendFd(fd));
        }
        let fd = files.iter().position(Option::is_none)?;
        files[fd] = Some(File { kind, handle, pos: 0, opens: 1 });
        Some(BackendFd(fd))
    });
    fd.ok_or_else(|| {
        let _ = device.close(handle);
        Error::TooMany
    })
}

fn with_file<R>(fd: BackendFd, f: impl FnOnce(&mut File) -> R) -> Result<R> {
    FILES.with(|files| files.get_mut(fd.0).and_then(Option::as_mut).map(f).ok_or(Error::Invalid))
}

#[derive(Clone, Copy)]
struct File {
    kind: usize, // in DEVICES
    handle: usize,
    pos: u64,
    opens: u32,
}

impl File {
    fn device(&self) -> &'static dyn Device {
        DEVICES[self.kind]
    }

    fn advance(&self, fd: BackendFd, count: usize) -> Result<usize> {
        if self.device().seekable() {
            with_file(fd, |f| f.pos += count as u64)?;
        }
        Ok(count)
    }
}
