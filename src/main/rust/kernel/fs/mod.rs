// SPDX-License-Identifier: MIT

pub mod devfs;
pub mod ext2;
pub mod p9;
pub mod sysfs;
pub mod tmpfs;
pub mod vfs;

pub use crate::api::fs::{Kind, LABEL_LENGTH, ListEntry, Medium, PATH_LENGTH, Stat};
pub use crate::api::{Error, Result};

use crate::api::fs::{O_APPEND, O_CREATE, O_NONBLOCK, O_READ, O_TRUNCATE, O_WRITE};
use crate::mem::realloc;
use crate::text::Text;
use core::ffi::{c_char, c_int};

pub type MountText = Text<MOUNT_PATH_LENGTH>;

pub type Label = Text<{ LABEL_LENGTH + 1 }>;

pub type Name = Text<16>;

pub struct MountInfo {
    pub path: MountText,
    pub label: MountText,
    pub index: usize, // meaning depends on Medium: drive index, 9p instance, 0 otherwise.
    pub medium: Medium,
    pub generation: u32,
    pub total: u64,
    pub free: u64,
    pub writable: bool,
    pub unmounted: bool,
}

pub struct Listing {
    entries: *mut ListEntry,
    count: usize,
    entries_capacity: usize,
    names: *mut u8,
    names_length: usize,
    names_capacity: usize,
}

impl Listing {
    pub const EMPTY: Self = Self {
        entries: core::ptr::null_mut(),
        count: 0,
        entries_capacity: 0,
        names: core::ptr::null_mut(),
        names_length: 0,
        names_capacity: 0,
    };

    pub fn into_raw(self) -> (*mut ListEntry, usize, *mut c_char) {
        for i in 0..self.count {
            let entry = unsafe { &mut *self.entries.add(i) };
            entry.name = unsafe { self.names.add(entry.name.addr()) }.cast();
        }
        (self.entries, self.count, self.names.cast())
    }

    pub fn add(&mut self, name: &[u8], kind: Kind) -> Result<()> {
        if name.len() > 255 {
            return Err(Error::NameTooLong);
        }
        grow(&mut self.names, &mut self.names_capacity, self.names_length + name.len() + 1, 256)?;
        grow(&mut self.entries, &mut self.entries_capacity, self.count + 1, 16)?;
        unsafe {
            let at = self.names.add(self.names_length);
            at.copy_from_nonoverlapping(name.as_ptr(), name.len());
            *at.add(name.len()) = 0;
            let name_length = name.len() as u8;
            let name = core::ptr::without_provenance(self.names_length);
            self.entries.add(self.count).write(ListEntry { kind, name_length, name });
        }
        self.names_length += name.len() + 1;
        self.count += 1;
        Ok(())
    }

    pub fn add_unique(&mut self, name: &[u8], kind: Kind) -> Result<()> {
        for i in 0..self.count {
            let entry = unsafe { &*self.entries.add(i) };
            let listed =
                unsafe { core::slice::from_raw_parts(self.names.add(entry.name.addr()), entry.name_length.into()) };
            if listed == name {
                return Ok(());
            }
        }
        self.add(name, kind)
    }
}

pub enum Data<'a> {
    Read(&'a mut [u8]),
    Write(&'a [u8]),
}

impl Data<'_> {
    pub fn len(&self) -> usize {
        match self {
            Data::Read(dst) => dst.len(),
            Data::Write(src) => src.len(),
        }
    }

    pub fn slice(&mut self, at: usize, len: usize) -> Data<'_> {
        match self {
            Data::Read(dst) => Data::Read(&mut dst[at..at + len]),
            Data::Write(src) => Data::Write(&src[at..at + len]),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct OpenFlags(pub c_int);

impl OpenFlags {
    pub const READ: Self = Self(O_READ);
    pub const WRITE: Self = Self(O_WRITE);
    pub const CREATE: Self = Self(O_CREATE);
    pub const TRUNCATE: Self = Self(O_TRUNCATE);
    pub const APPEND: Self = Self(O_APPEND);
    pub const NONBLOCK: Self = Self(O_NONBLOCK);
    pub const MODIFY: Self = Self(O_WRITE | O_CREATE | O_TRUNCATE | O_APPEND);

    pub fn has(self, flags: Self) -> bool {
        self.0 & flags.0 != 0
    }
}

pub(crate) const SEEK_SET: c_int = 0;
const SEEK_CUR: c_int = 1;
const SEEK_END: c_int = 2;

const MOUNT_PATH_LENGTH: usize = 48;

pub(crate) const MNT: &[u8] = b"/mnt/";

pub fn next_name(path: &[u8]) -> Option<(&[u8], &[u8])> {
    let path = trim_slashes(path);
    let end = path.iter().position(|&c| c == b'/').unwrap_or(path.len());
    (end > 0).then(|| path.split_at(end))
}

pub fn is_valid_name(name: &[u8]) -> bool {
    !name.is_empty()
        && name != b"."
        && name != b".."
        && name.iter().all(|&c| c.is_ascii_alphanumeric() || c == b'.' || c == b'_' || c == b'-')
        && !name.iter().all(u8::is_ascii_digit)
}

pub fn trim_slashes(path: &[u8]) -> &[u8] {
    &path[path.iter().position(|&c| c != b'/').unwrap_or(path.len())..]
}

pub fn trim_end(path: &[u8]) -> &[u8] {
    match path {
        [rest @ .., b'/'] if !rest.is_empty() => rest,
        _ => path,
    }
}

pub fn seek(pos: u64, offset: i64, whence: c_int, size: impl FnOnce() -> Result<u64>) -> Result<u64> {
    let base = match whence {
        SEEK_SET => 0,
        SEEK_CUR => pos,
        SEEK_END => size()?,
        _ => return Err(Error::Invalid),
    };
    base.checked_add_signed(offset).filter(|&p| p <= i64::MAX as u64).ok_or(Error::Invalid)
}

fn grow<T>(buffer: &mut *mut T, capacity: &mut usize, count: usize, step: usize) -> Result<()> {
    if count <= *capacity {
        return Ok(());
    }
    let grown = *capacity * 2 + step;
    let data = unsafe { realloc(buffer.cast(), grown * size_of::<T>()) }.cast::<T>();
    if data.is_null() {
        return Err(Error::NoSpace);
    }
    (*buffer, *capacity) = (data, grown);
    Ok(())
}
