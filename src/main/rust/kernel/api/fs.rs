// SPDX-License-Identifier: MIT

use super::{Error, Result, buffer, buffer_mut, bytes, code, int, long};
use crate::arch::KernelCell;
use crate::fs::vfs::{self, Fd};
use crate::fs::{self, MountInfo, OpenFlags};
use core::ffi::{c_char, c_int, c_long, c_void};

pub const O_READ: c_int = 1;
pub const O_WRITE: c_int = 2;
pub const O_CREATE: c_int = 4;
pub const O_TRUNCATE: c_int = 8;
pub const O_APPEND: c_int = 16;
pub const O_NONBLOCK: c_int = 32;

pub const POLL_IN: c_int = 1;
pub const POLL_OUT: c_int = 2;

pub const PATH_LENGTH: usize = 512;
pub const LABEL_LENGTH: usize = 16;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Default)]
pub enum Medium {
    #[default]
    Unknown,
    Blank,
    Unsupported, // ext2 that this kernel cannot use
    Ext2,
    P9,
    Tmpfs,
    Devfs,
    Sysfs,
}

#[repr(C)]
pub struct MountEntry {
    pub path: *const c_char,
    pub label: *const c_char,
    pub index: usize, // meaning depends on Medium: drive index, 9p instance, 0 otherwise.
    pub medium: Medium,
    pub generation: u32,
    pub total: u64,
    pub free: u64,
    pub writable: bool,
    pub unmounted: bool,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    File,
    Directory,
    Link,
    Device,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Stat {
    pub size: u64,
    pub mtime: u32,
    pub kind: Kind,
}

#[repr(C)]
pub struct ListEntry {
    pub kind: Kind,
    pub name_length: u8,
    pub name: *const c_char,
}

#[repr(C)]
pub struct Listing {
    entries: *mut ListEntry,
    count: usize,
    names: *mut c_char, // backing storage fo ListEntry::name
}

static ENTRY: KernelCell<Option<MountInfo>> = KernelCell::new(None);

#[unsafe(no_mangle)]
extern "C" fn fs_mount_count() -> c_int {
    vfs::mount_count() as c_int
}

/// The strings in `out` are only valid until the next file system call.
#[unsafe(no_mangle)]
extern "C" fn fs_mount_entry(index: c_int, out: &mut MountEntry) -> bool {
    let Some(info) = usize::try_from(index).ok().and_then(vfs::mount_info) else { return false };
    *out = ENTRY.with(|entry| {
        let info = entry.insert(info);
        MountEntry {
            path: info.path.as_ptr(),
            label: info.label.as_ptr(),
            index: info.index,
            medium: info.medium,
            generation: info.generation,
            total: info.total,
            free: info.free,
            writable: info.writable,
            unmounted: info.unmounted,
        }
    });
    true
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fs_unmount(path: *const c_char, generation: u32) -> c_int {
    code(vfs::unmount_drive(unsafe { bytes(path) }, generation))
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fs_mount(path: *const c_char) -> c_int {
    code(vfs::mount_drive(unsafe { bytes(path) }))
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fs_info(path: *const c_char, total: &mut u64, free: &mut u64) -> c_int {
    code(vfs::info(unsafe { bytes(path) }).map(|info| (*total, *free) = info))
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fs_stat(path: *const c_char, st: &mut Stat) -> c_int {
    *st = Stat::default();
    code(vfs::stat(unsafe { bytes(path) }).map(|s| *st = s))
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fs_lstat(path: *const c_char, st: &mut Stat) -> c_int {
    *st = Stat::default();
    code(vfs::lstat(unsafe { bytes(path) }).map(|s| *st = s))
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fs_readlink(path: *const c_char, dst: *mut c_char, len: usize) -> c_long {
    long(vfs::readlink(unsafe { bytes(path) }, unsafe { buffer_mut(dst.cast(), len) }))
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fs_realpath(path: *const c_char, out: *mut c_char) -> c_int {
    let mut real = [0; PATH_LENGTH];
    code(vfs::realpath(unsafe { bytes(path) }, &mut real).map(|len| unsafe { copy_with_nul(&real[..len], out) }))
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fs_symlink(target: *const c_char, path: *const c_char) -> c_int {
    code(vfs::symlink(unsafe { bytes(target) }, unsafe { bytes(path) }))
}

/// The caller must always free out->entries and out->names.
#[unsafe(no_mangle)]
unsafe extern "C" fn fs_list(path: *const c_char, out: &mut Listing) -> c_int {
    let mut listing = fs::Listing::EMPTY;
    let result = vfs::list(unsafe { bytes(path) }, &mut listing);
    (out.entries, out.count, out.names) = listing.into_raw();
    code(result)
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fs_label(path: *const c_char, label: *mut c_char) -> c_int {
    code(vfs::label(unsafe { bytes(path) }).map(|l| unsafe { copy_with_nul(l.as_bytes(), label) }))
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fs_open(path: *const c_char, flags: c_int) -> c_int {
    int(vfs::open(unsafe { bytes(path) }, OpenFlags(flags)).map(|fd| fd.0))
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fs_mkdir(path: *const c_char) -> c_int {
    code(vfs::mkdir(unsafe { bytes(path) }))
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fs_link(from: *const c_char, to: *const c_char) -> c_int {
    code(vfs::link(unsafe { bytes(from) }, unsafe { bytes(to) }))
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fs_rename(from: *const c_char, to: *const c_char) -> c_int {
    code(vfs::rename(unsafe { bytes(from) }, unsafe { bytes(to) }))
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fs_remove(path: *const c_char) -> c_int {
    code(vfs::remove(unsafe { bytes(path) }))
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fs_format(path: *const c_char, label: *const c_char, generation: u32) -> c_int {
    let label = (!label.is_null()).then(|| unsafe { bytes(label) });
    code(vfs::format(unsafe { bytes(path) }, label, generation))
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fs_set_label(path: *const c_char, label: *const c_char, generation: u32) -> c_int {
    code(vfs::set_label(unsafe { bytes(path) }, unsafe { bytes(label) }, generation))
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fs_read(fd: c_int, dst: *mut c_void, len: usize) -> c_long {
    long(handle(fd).and_then(|h| vfs::read(h, unsafe { buffer_mut(dst.cast(), len) })))
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fs_write(fd: c_int, src: *const c_void, len: usize) -> c_long {
    long(handle(fd).and_then(|h| vfs::write(h, unsafe { buffer(src.cast(), len) })))
}

#[unsafe(no_mangle)]
extern "C" fn fs_seek(fd: c_int, offset: c_long, whence: c_int) -> c_long {
    long(handle(fd).and_then(|h| vfs::seek(h, offset, whence)))
}

#[unsafe(no_mangle)]
extern "C" fn fs_poll(fd: c_int) -> c_int {
    let poll = handle(fd).and_then(vfs::poll);
    poll.map_or_else(|e| e as c_int, |p| if p.readable { POLL_IN } else { 0 } | if p.writable { POLL_OUT } else { 0 })
}

#[unsafe(no_mangle)]
extern "C" fn fs_close(fd: c_int) -> c_int {
    code(handle(fd).and_then(vfs::close))
}

unsafe fn copy_with_nul(s: &[u8], out: *mut c_char) {
    unsafe {
        out.copy_from_nonoverlapping(s.as_ptr().cast(), s.len());
        *out.add(s.len()) = 0;
    }
}

fn handle(fd: c_int) -> Result<Fd> {
    usize::try_from(fd).map(Fd).map_err(|_| Error::BadFd)
}
