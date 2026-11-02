// SPDX-License-Identifier: MIT

use super::vfs::{BackendFd, FileSystem, MountId};
use super::{Error, Kind, Listing, Name, OpenFlags, Result, Stat, next_name};
use crate::arch::KernelCell;
use crate::drivers::fb::Framebuffers;
use crate::drivers::rtc::Clocks;
use crate::drivers::uart::Ttys;
use crate::drivers::virtio::console::Ports;
use crate::drivers::virtio::input::Keyboards;
use core::ffi::c_int;

pub const MAX_FILES: usize = 4;

pub type Content = crate::text::Text<CONTENT_SIZE>;

pub trait Class: Sync {
    fn name(&self) -> &'static [u8];

    fn count(&self) -> usize;

    fn instance(&self, index: usize) -> Option<Name>;

    fn find(&self, name: &[u8]) -> Option<usize> {
        (0..self.count()).find(|&i| self.instance(i).is_some_and(|n| n.as_bytes() == name))
    }

    fn attributes(&self) -> &'static [&'static [u8]];

    fn show(&self, index: usize, attribute: usize, out: &mut Content) -> Option<()>;
}

pub struct Sysfs;

impl FileSystem for Sysfs {
    fn stat(&self, _: MountId, path: &[u8]) -> Result<Stat> {
        match lookup(path).ok_or(Error::NotFound)? {
            Node::File(content) => Ok(Stat { size: content.len() as u64, ..Stat::default() }),
            _ => Ok(Stat { kind: Kind::Directory, ..Stat::default() }),
        }
    }

    fn list(&self, _: MountId, path: &[u8], out: &mut Listing) -> Result<()> {
        match lookup(path).ok_or(Error::NotFound)? {
            Node::Root => out.add(b"class", Kind::Directory),
            Node::Classes => CLASSES.iter().try_for_each(|class| out.add(class.name(), Kind::Directory)),
            Node::Class(class) => (0..class.count())
                .filter_map(|i| class.instance(i))
                .try_for_each(|name| out.add(name.as_bytes(), Kind::Directory)),
            Node::Instance(class) => class.attributes().iter().try_for_each(|name| out.add(name, Kind::File)),
            Node::File(_) => Err(Error::NotDir),
        }
    }

    fn open(&self, _: MountId, path: &[u8], flags: OpenFlags) -> Result<BackendFd> {
        if flags.has(OpenFlags::MODIFY) {
            return Err(Error::ReadOnly);
        }
        FILES.with(|files| {
            let fd = files.iter().position(Option::is_none).ok_or(Error::TooMany)?;
            match lookup(path).ok_or(Error::NotFound)? {
                Node::File(content) => {
                    files[fd] = Some(File { content, position: 0 });
                    Ok(BackendFd(fd))
                }
                _ => Err(Error::IsDir),
            }
        })
    }

    fn read(&self, fd: BackendFd, dst: &mut [u8]) -> Result<usize> {
        with_file(fd, |file| {
            let src = &file.content.as_bytes()[file.position..];
            let count = dst.len().min(src.len());
            dst[..count].copy_from_slice(&src[..count]);
            file.position += count;
            Ok(count)
        })
    }

    fn seek(&self, fd: BackendFd, offset: i64, whence: c_int) -> Result<usize> {
        with_file(fd, |file| {
            let len = file.content.len() as u64;
            let position = super::seek(file.position as u64, offset, whence, || Ok(len))?;
            if position > len {
                return Err(Error::Invalid);
            }
            file.position = position as usize;
            Ok(file.position)
        })
    }

    fn close(&self, fd: BackendFd) -> Result<()> {
        FILES.with(|files| files.get_mut(fd.0).and_then(Option::take).map(|_| ()).ok_or(Error::Invalid))
    }
}

const CONTENT_SIZE: usize = 72;
const MAX_DEPTH: usize = 4;
const CLASSES: &[&dyn Class] = &[&Framebuffers, &Keyboards, &Clocks, &Ttys, &Ports];
static FILES: KernelCell<[Option<File>; MAX_FILES]> = KernelCell::new([None; MAX_FILES]);

fn split(mut path: &[u8]) -> Option<([&[u8]; MAX_DEPTH], usize)> {
    let mut names: [&[u8]; MAX_DEPTH] = [&[]; MAX_DEPTH];
    let mut depth = 0;
    while let Some((name, rest)) = next_name(path) {
        *names.get_mut(depth)? = name;
        depth += 1;
        path = rest;
    }
    Some((names, depth))
}

fn lookup(path: &[u8]) -> Option<Node> {
    let (names, depth) = split(path)?;
    if depth == 0 {
        return Some(Node::Root);
    }
    if names[0] != b"class" {
        return None;
    }
    if depth == 1 {
        return Some(Node::Classes);
    }
    let class = *CLASSES.iter().find(|class| class.name() == names[1])?;
    if depth == 2 {
        return Some(Node::Class(class));
    }
    let index = class.find(names[2])?;
    if depth == 3 {
        return Some(Node::Instance(class));
    }
    let attribute = class.attributes().iter().position(|&name| name == names[3])?;
    let mut content = Content::EMPTY;
    class.show(index, attribute, &mut content)?;
    content.push(b"\n");
    Some(Node::File(content))
}

fn with_file<R>(fd: BackendFd, f: impl FnOnce(&mut File) -> Result<R>) -> Result<R> {
    FILES.with(|files| f(files.get_mut(fd.0).and_then(Option::as_mut).ok_or(Error::Invalid)?))
}

enum Node {
    Root,
    Classes,
    Class(&'static dyn Class),
    Instance(&'static dyn Class),
    File(Content),
}

#[derive(Clone, Copy)]
struct File {
    content: Content,
    position: usize,
}
