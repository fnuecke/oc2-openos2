// SPDX-License-Identifier: MIT

use super::vfs::{self, BackendFd, FileSystem, MountId};
use super::{Error, Kind, Listing, Medium, MountInfo, MountText, OpenFlags, Result, Stat, trim_end, trim_slashes};
use crate::api::fs::{O_READ, O_WRITE};
use crate::arch::KernelCell;
use crate::drivers::rtc;
use crate::mem::{HeapBuf, heap_total};
use core::ffi::c_int;

pub const MAX_FILES: usize = 8;
pub const PATH: &[u8] = b"/tmp";

pub struct Tmpfs;

impl FileSystem for Tmpfs {
    fn stat(&self, _: MountId, path: &[u8]) -> Result<Stat> {
        STATE.with(|s| {
            let name = key(path);
            if name.is_empty() {
                return Ok(Stat { kind: Kind::Directory, ..Stat::default() });
            }
            let node = s.node(s.lookup(path)?);
            Ok(Stat { size: node.size as u64, mtime: node.mtime, kind: node.kind })
        })
    }

    fn readlink(&self, _: MountId, path: &[u8], out: &mut [u8]) -> Result<usize> {
        STATE.with(|s| {
            if key(path).is_empty() {
                return Err(Error::Invalid);
            }
            let node = s.node(s.lookup(path)?);
            if node.kind != Kind::Link {
                return Err(Error::Invalid);
            }
            let len = node.size.min(out.len());
            out[..len].copy_from_slice(&node.data[..len]);
            Ok(len)
        })
    }

    fn list(&self, _: MountId, path: &[u8], out: &mut Listing) -> Result<()> {
        STATE.with(|s| {
            let dir = key(path);
            s.check_dir(dir)?;
            for node in s.nodes().iter().flatten() {
                if let Some(name) = as_child_name_of(node.name(), dir).filter(|name| !name.contains(&b'/')) {
                    out.add(name, node.kind)?;
                }
            }
            Ok(())
        })
    }

    fn open(&self, _: MountId, path: &[u8], flags: OpenFlags) -> Result<BackendFd> {
        STATE.with(|s| {
            let fd = s.files.iter().position(Option::is_none).ok_or(Error::TooMany)?;
            let name = key(path);
            if name.is_empty() || path.last() == Some(&b'/') && s.find(name).is_none() {
                return Err(Error::IsDir);
            }
            let index = match s.lookup(path) {
                Err(Error::NotFound) if flags.has(OpenFlags::CREATE) => s.create(name, Kind::File)?,
                result => result?,
            };
            let node = s.node(index);
            match node.kind {
                Kind::Directory => return Err(Error::IsDir),
                Kind::Link => return Err(link_at(name.len())),
                _ => {}
            }
            if flags.has(OpenFlags::TRUNCATE) && flags.has(OpenFlags::WRITE) {
                node.truncate();
            }
            node.opens += 1;
            s.files[fd] = Some(File { node: index, pos: 0, flags });
            Ok(BackendFd(fd))
        })
    }

    fn read(&self, fd: BackendFd, dst: &mut [u8]) -> Result<usize> {
        STATE.with(|s| {
            let file = s.file(fd)?;
            if !file.flags.has(OpenFlags::READ) {
                return Err(Error::BadFd);
            }
            let node = s.node(file.node);
            let src = &node.data[file.pos.min(node.size)..node.size];
            let count = dst.len().min(src.len());
            dst[..count].copy_from_slice(&src[..count]);
            s.files[fd.0].as_mut().unwrap().pos += count;
            Ok(count)
        })
    }

    fn write(&self, fd: BackendFd, src: &[u8]) -> Result<usize> {
        STATE.with(|s| {
            let file = s.file(fd)?;
            if !file.flags.has(OpenFlags::WRITE) {
                return Err(Error::BadFd);
            }
            let pos = if file.flags.has(OpenFlags::APPEND) { s.node(file.node).size } else { file.pos };
            if src.is_empty() {
                return Ok(0);
            }
            let room = s.reserve(file.node, pos.checked_add(src.len()).ok_or(Error::Invalid)?);
            let count = room.saturating_sub(pos).min(src.len());
            if count == 0 {
                return Err(Error::NoSpace);
            }
            let node = s.node(file.node);
            if pos > node.size {
                node.data[node.size..pos].fill(0);
            }
            node.data[pos..pos + count].copy_from_slice(&src[..count]);
            node.size = node.size.max(pos + count);
            node.mtime = now();
            s.files[fd.0].as_mut().unwrap().pos = pos + count;
            Ok(count)
        })
    }

    fn seek(&self, fd: BackendFd, offset: i64, whence: c_int) -> Result<usize> {
        STATE.with(|s| {
            let file = s.file(fd)?;
            let size = s.node(file.node).size as u64;
            let pos = super::seek(file.pos as u64, offset, whence, || Ok(size))? as usize;
            s.files[fd.0].as_mut().unwrap().pos = pos;
            Ok(pos)
        })
    }

    fn close(&self, fd: BackendFd) -> Result<()> {
        STATE.with(|s| {
            let file = s.files.get_mut(fd.0).and_then(Option::take).ok_or(Error::Invalid)?;
            s.node(file.node).opens -= 1;
            s.release(file.node);
            Ok(())
        })
    }

    fn mkdir(&self, _: MountId, path: &[u8]) -> Result<()> {
        STATE.with(|s| {
            let name = key(path);
            if name.is_empty() || s.find(name).is_some() {
                return Err(Error::Exists);
            }
            s.create(name, Kind::Directory).map(|_| ())
        })
    }

    fn rename(&self, _: MountId, from_path: &[u8], to: &[u8]) -> Result<()> {
        STATE.with(|s| {
            let (from, to) = (key(from_path), key(to));
            if from.is_empty() || to.is_empty() {
                return Err(Error::Busy);
            }
            let source = s.lookup(from_path)?;
            if from == to {
                return Ok(());
            }
            let is_dir = s.is_dir(source);
            if is_dir && as_child_name_of(to, from).is_some() {
                return Err(Error::Invalid);
            }
            s.check_new(to)?;
            let replaced = s.find(to);
            if let Some(replaced) = replaced {
                match (is_dir, s.is_dir(replaced)) {
                    (false, true) => return Err(Error::IsDir),
                    (true, false) => return Err(Error::NotDir),
                    (true, true) if s.has_entries(to) => return Err(Error::NotEmpty),
                    _ => {}
                }
            }
            let moved = |node: &Node| node.name() == from || as_child_name_of(node.name(), from).is_some();
            for node in s.nodes_mut().iter_mut().flatten().filter(|node| moved(node)) {
                let len = node.name_len - from.len() + to.len();
                if node.name.len() < len && !node.name.resize(len) {
                    return Err(Error::NoSpace);
                }
            }
            if let Some(replaced) = replaced {
                s.unname(replaced);
            }
            for node in s.nodes_mut().iter_mut().flatten().filter(|node| moved(node)) {
                node.name.copy_within(from.len()..node.name_len, to.len());
                node.name[..to.len()].copy_from_slice(to);
                node.name_len = node.name_len - from.len() + to.len();
            }
            Ok(())
        })
    }

    fn remove(&self, _: MountId, path: &[u8]) -> Result<()> {
        STATE.with(|s| {
            let name = key(path);
            if name.is_empty() {
                return Err(Error::Busy);
            }
            let index = s.lookup(path)?;
            if s.is_dir(index) && s.has_entries(name) {
                return Err(Error::NotEmpty);
            }
            s.unname(index);
            Ok(())
        })
    }

    fn symlink(&self, _: MountId, path: &[u8], target: &[u8]) -> Result<()> {
        STATE.with(|s| {
            let name = key(path);
            if name.is_empty() || s.find(name).is_some() {
                return Err(Error::Exists);
            }
            let index = s.create(name, Kind::Link)?;
            if s.reserve(index, target.len()) < target.len() {
                s.unname(index);
                return Err(Error::NoSpace);
            }
            let node = s.node(index);
            node.data[..target.len()].copy_from_slice(target);
            node.size = target.len();
            Ok(())
        })
    }

    fn info(&self, _: MountId) -> Result<(u64, u64)> {
        STATE.with(|s| Ok((s.limit as u64, s.limit.saturating_sub(s.used()) as u64)))
    }
}

const MAX_NAME: usize = 255;

static STATE: KernelCell<State> = KernelCell::new(State { slots: HeapBuf::EMPTY, files: [None; MAX_FILES], limit: 0 });

pub fn mount() {
    STATE.with(|s| s.limit = heap_total() / 2);
    vfs::mount(PATH, &Tmpfs, MountId(0)).expect("the table has room");
}

pub fn mount_info() -> MountInfo {
    let (total, free) = Tmpfs.info(MountId(0)).unwrap_or((0, 0));
    MountInfo {
        path: MountText::of(PATH),
        label: MountText::of(b"tmpfs"),
        index: 0,
        medium: Medium::Tmpfs,
        generation: 0,
        total,
        free,
        writable: true,
        unmounted: false,
    }
}

pub fn open_anonymous() -> Result<BackendFd> {
    STATE.with(|s| {
        let fd = s.files.iter().position(Option::is_none).ok_or(Error::TooMany)?;
        let index = s.add(b"", Kind::File)?;
        s.node(index).opens = 1;
        s.files[fd] = Some(File { node: index, pos: 0, flags: OpenFlags(O_READ | O_WRITE) });
        Ok(BackendFd(fd))
    })
}

fn now() -> u32 {
    rtc::seconds() as u32
}

fn key(path: &[u8]) -> &[u8] {
    trim_end(trim_slashes(path))
}

fn link_at(end: usize) -> Error {
    vfs::link_at(1 + end) // vfs passes the path in /tmp as "/" and the name
}

fn as_child_name_of<'a>(name: &'a [u8], dir: &[u8]) -> Option<&'a [u8]> {
    if dir.is_empty() {
        return (!name.is_empty()).then_some(name);
    }
    name.strip_prefix(dir)?.strip_prefix(b"/")
}

fn split(name: &[u8]) -> (&[u8], &[u8]) {
    match name.iter().rposition(|&c| c == b'/') {
        Some(at) => (&name[..at], &name[at + 1..]),
        None => (&[], name),
    }
}

struct State {
    slots: HeapBuf, // of Option<Node>
    files: [Option<File>; MAX_FILES],
    limit: usize,
}

impl State {
    fn nodes(&self) -> &[Option<Node>] {
        if self.slots.is_empty() {
            return &[];
        }
        unsafe { core::slice::from_raw_parts(self.slots.as_ptr().cast(), self.slots.len() / size_of::<Option<Node>>()) }
    }

    fn nodes_mut(&mut self) -> &mut [Option<Node>] {
        if self.slots.is_empty() {
            return &mut [];
        }
        let count = self.slots.len() / size_of::<Option<Node>>();
        unsafe { core::slice::from_raw_parts_mut(self.slots.as_mut_ptr().cast(), count) }
    }

    fn node(&mut self, index: usize) -> &mut Node {
        self.nodes_mut()[index].as_mut().unwrap()
    }

    fn find(&self, name: &[u8]) -> Option<usize> {
        self.nodes().iter().position(|n| n.as_ref().is_some_and(|n| n.name() == name))
    }

    fn lookup(&self, path: &[u8]) -> Result<usize> {
        let name = key(path);
        match self.find(name) {
            Some(index) if path.last() == Some(&b'/') && !self.is_dir(index) => Err(self.y_not_dir_tho(name)),
            Some(index) => Ok(index),
            None => self.check_dir(split(name).0).and(Err(Error::NotFound)),
        }
    }

    fn is_dir(&self, index: usize) -> bool {
        self.kind(index) == Kind::Directory
    }

    fn kind(&self, index: usize) -> Kind {
        self.nodes()[index].as_ref().map_or(Kind::File, |node| node.kind)
    }

    fn file(&self, fd: BackendFd) -> Result<File> {
        self.files.get(fd.0).copied().flatten().ok_or(Error::Invalid)
    }

    fn check_dir(&self, dir: &[u8]) -> Result<()> {
        if dir.is_empty() {
            return Ok(());
        }
        match self.find(dir) {
            Some(index) if self.is_dir(index) => Ok(()),
            _ => Err(self.y_not_dir_tho(dir)),
        }
    }

    fn y_not_dir_tho(&self, dir: &[u8]) -> Error {
        let ends = dir.iter().enumerate().filter(|&(_, &c)| c == b'/').map(|(at, _)| at).chain([dir.len()]);
        for end in ends {
            match self.find(&dir[..end]).map(|index| self.kind(index)) {
                Some(Kind::Directory) => {}
                Some(Kind::Link) => return link_at(end),
                Some(_) => return Error::NotDir,
                None => return Error::NotFound,
            }
        }
        Error::NotDir
    }

    fn check_new(&self, name: &[u8]) -> Result<()> {
        let (dir, last) = split(name);
        if last.len() > MAX_NAME {
            return Err(Error::NameTooLong);
        }
        self.check_dir(dir)
    }

    fn has_entries(&self, dir: &[u8]) -> bool {
        self.nodes().iter().flatten().any(|node| as_child_name_of(node.name(), dir).is_some())
    }

    fn create(&mut self, name: &[u8], kind: Kind) -> Result<usize> {
        self.check_new(name)?;
        self.add(name, kind)
    }

    fn add(&mut self, name: &[u8], kind: Kind) -> Result<usize> {
        if self.used() + size_of::<Option<Node>>() + name.len() > self.limit {
            return Err(Error::NoSpace);
        }
        let mut node = Node {
            name: HeapBuf::EMPTY,
            name_len: name.len(),
            data: HeapBuf::EMPTY,
            size: 0,
            mtime: now(),
            kind,
            opens: 0,
        };
        if !name.is_empty() {
            node.name = HeapBuf::new(name.len()).ok_or(Error::NoSpace)?;
            node.name.copy_from_slice(name);
        }
        let index = self.free_slot()?;
        self.nodes_mut()[index] = Some(node);
        Ok(index)
    }

    fn free_slot(&mut self) -> Result<usize> {
        if let Some(index) = self.nodes().iter().position(Option::is_none) {
            return Ok(index);
        }
        let count = self.nodes().len();
        let grown = (count * 2).max(8);
        if !self.slots.resize(grown * size_of::<Option<Node>>()) {
            return Err(Error::NoSpace);
        }
        let slots = self.slots.as_mut_ptr().cast::<Option<Node>>();
        for index in count..grown {
            unsafe { slots.add(index).write(None) };
        }
        Ok(count)
    }

    fn reserve(&mut self, index: usize, len: usize) -> usize {
        let room = self.node(index).data.len();
        if len <= room {
            return room;
        }
        let max = room + self.limit.saturating_sub(self.used());
        let data = &mut self.node(index).data;
        for wanted in [len.max(room * 2), len, max] {
            if wanted > room && wanted <= max && data.resize(wanted) {
                return wanted;
            }
        }
        room
    }

    fn used(&self) -> usize {
        self.nodes().iter().flatten().map(|node| size_of::<Option<Node>>() + node.name.len() + node.data.len()).sum()
    }

    fn unname(&mut self, index: usize) {
        let node = self.node(index);
        node.name = HeapBuf::EMPTY;
        node.name_len = 0;
        self.release(index);
    }

    fn release(&mut self, index: usize) {
        let slot = &mut self.nodes_mut()[index];
        if slot.as_ref().is_some_and(|node| node.name_len == 0 && node.opens == 0) {
            *slot = None;
        }
    }
}

struct Node {
    name: HeapBuf,
    name_len: usize,
    data: HeapBuf,
    size: usize,
    mtime: u32,
    kind: Kind,
    opens: u32,
}

impl Node {
    fn name(&self) -> &[u8] {
        &self.name[..self.name_len]
    }

    fn truncate(&mut self) {
        self.data = HeapBuf::EMPTY;
        self.size = 0;
        self.mtime = now();
    }
}

#[derive(Clone, Copy)]
struct File {
    node: usize,
    pos: usize,
    flags: OpenFlags,
}
