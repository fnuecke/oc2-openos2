// SPDX-License-Identifier: MIT

mod client;

use super::vfs::{self, BackendFd, FileSystem, MountId};
use super::{Error, Listing, Medium, MountInfo, MountText, OpenFlags, Result, Stat};
use crate::arch::KernelCell;
use crate::drivers::virtio::{Device, Driver};
use client::{DIR_BUFFER, Fid, Mount};
use core::ffi::c_int;

pub const DRIVER: Driver = Driver { id: 9, probe, keeps_config_change: false, interrupt_hook: None };
pub const MAX_FILES: usize = 8;
pub const MAX_MOUNTS: usize = 2;

pub struct P9;

impl FileSystem for P9 {
    fn stat(&self, mount: MountId, path: &[u8]) -> Result<Stat> {
        STATE.with(|s| s.mount(mount).stat(path))
    }

    fn list(&self, mount: MountId, path: &[u8], out: &mut Listing) -> Result<()> {
        STATE.with(|s| {
            let m = unsafe { &mut *s.mounts[..s.count][mount.0] };
            m.list(path, &mut s.dir_buffer, out)
        })
    }

    fn open(&self, mount: MountId, path: &[u8], flags: OpenFlags) -> Result<BackendFd> {
        STATE.with(|s| {
            let fd = s.files.iter().position(Option::is_none).ok_or(Error::TooMany)?;
            if path.last() == Some(&b'/') {
                return Err(Error::IsDir);
            }
            let fid = s.mount(mount).open(path, flags)?;
            s.files[fd] = Some(File { mount, fid, pos: 0, flags });
            Ok(BackendFd(fd))
        })
    }

    fn read(&self, fd: BackendFd, dst: &mut [u8]) -> Result<usize> {
        STATE.with(|s| {
            let (m, file) = s.file(fd)?;
            if !file.flags.has(OpenFlags::READ) {
                return Err(Error::BadFd);
            }
            m.read(file.fid, &mut file.pos, dst)
        })
    }

    fn write(&self, fd: BackendFd, src: &[u8]) -> Result<usize> {
        STATE.with(|s| {
            let (m, file) = s.file(fd)?;
            if !file.flags.has(OpenFlags::WRITE) {
                return Err(Error::BadFd);
            }
            m.write(file.fid, &mut file.pos, file.flags.has(OpenFlags::APPEND), src)
        })
    }

    fn seek(&self, fd: BackendFd, offset: i64, whence: c_int) -> Result<usize> {
        STATE.with(|s| {
            let (m, file) = s.file(fd)?;
            file.pos = super::seek(file.pos, offset, whence, || Ok(m.getattr(file.fid)?.size))?;
            Ok(file.pos as usize)
        })
    }

    fn close(&self, fd: BackendFd) -> Result<()> {
        STATE.with(|s| {
            let (m, file) = s.file(fd)?;
            let fid = file.fid;
            m.clunk(fid);
            s.files[fd.0] = None;
            Ok(())
        })
    }

    fn mkdir(&self, mount: MountId, path: &[u8]) -> Result<()> {
        STATE.with(|s| s.mount(mount).mkdir(path))
    }

    fn rename(&self, mount: MountId, from: &[u8], to: &[u8]) -> Result<()> {
        STATE.with(|s| s.mount(mount).rename(from, to))
    }

    fn remove(&self, mount: MountId, path: &[u8]) -> Result<()> {
        STATE.with(|s| s.mount(mount).remove(path))
    }

    fn info(&self, mount: MountId) -> Result<(u64, u64)> {
        STATE.with(|s| s.mount(mount).info())
    }
}

static STATE: KernelCell<State> = KernelCell::new(State {
    mounts: [core::ptr::null_mut(); MAX_MOUNTS],
    count: 0,
    files: [None; MAX_FILES],
    dir_buffer: [0; DIR_BUFFER],
});

pub fn mount_count() -> usize {
    STATE.with(|s| s.count)
}

pub fn mount_info(index: usize) -> MountInfo {
    STATE.with(|s| {
        let m = s.mount(MountId(index));
        let (total, free) = m.info().unwrap_or((0, 0));
        MountInfo {
            path: m.path,
            label: MountText::of(m.tag()),
            index,
            medium: Medium::P9,
            generation: 0,
            total,
            free,
            writable: !m.read_only,
            unmounted: false,
        }
    })
}

fn probe(device: Device) -> bool {
    STATE.with(|s| {
        if s.count == MAX_MOUNTS {
            return false;
        }
        let Some(m) = client::attach(device) else { return false };
        // Name dupes are dropped.
        if vfs::mount(unsafe { (*m).path }.as_bytes(), &P9, MountId(s.count)).is_err() {
            return false;
        }
        s.mounts[s.count] = m;
        s.count += 1;
        true
    })
}

#[derive(Clone, Copy)]
struct File {
    mount: MountId,
    fid: Fid,
    pos: u64,
    flags: OpenFlags,
}

struct State {
    mounts: [*mut Mount; MAX_MOUNTS],
    count: usize,
    files: [Option<File>; MAX_FILES],
    dir_buffer: [u8; DIR_BUFFER],
}

impl State {
    fn mount(&mut self, mount: MountId) -> &mut Mount {
        unsafe { &mut *self.mounts[..self.count][mount.0] }
    }

    fn file(&mut self, fd: BackendFd) -> Result<(&mut Mount, &mut File)> {
        let file = self.files.get_mut(fd.0).and_then(Option::as_mut);
        let file = file.ok_or(Error::Invalid)?;
        Ok((unsafe { &mut *self.mounts[file.mount.0] }, file))
    }
}
