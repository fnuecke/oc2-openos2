// SPDX-License-Identifier: MIT

use super::vfs::{BackendFd, FileSystem, MountId};
use super::{Error, Kind, Listing, Name, OpenFlags, Result, Stat, next_name};
use crate::arch::KernelCell;
use crate::board::board;
use crate::fdt;
use core::ffi::c_int;

registry!(pub CLASSES: &'static dyn Class);

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
            Node::File(value) => Ok(Stat { size: value.as_bytes().len() as u64, ..Stat::default() }),
            _ => Ok(Stat { kind: Kind::Directory, ..Stat::default() }),
        }
    }

    fn list(&self, _: MountId, path: &[u8], out: &mut Listing) -> Result<()> {
        match lookup(path).ok_or(Error::NotFound)? {
            Node::Root => out.add(b"class", Kind::Directory).and_then(|_| out.add(b"firmware", Kind::Directory)),
            Node::Classes => CLASSES.iter().try_for_each(|class| out.add(class.name(), Kind::Directory)),
            Node::Class(class) => (0..class.count())
                .filter_map(|i| class.instance(i))
                .try_for_each(|name| out.add(name.as_bytes(), Kind::Directory)),
            Node::Instance(class) => class.attributes().iter().try_for_each(|name| out.add(name, Kind::File)),
            Node::Firmware => out.add(b"devicetree", Kind::Directory).and_then(|_| out.add(b"fdt", Kind::File)),
            Node::Devicetree => out.add(b"base", Kind::Directory),
            Node::Tree(node) => {
                node.props().try_for_each(|(name, _)| out.add(name, Kind::File))?;
                if node.prop(b"name").is_none() {
                    out.add(b"name", Kind::File)?;
                }
                node.children().try_for_each(|child| out.add(child.name, Kind::Directory))
            }
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
                Node::File(value) => {
                    files[fd] = Some(File { value, position: 0 });
                    Ok(BackendFd(fd))
                }
                _ => Err(Error::IsDir),
            }
        })
    }

    fn read(&self, fd: BackendFd, dst: &mut [u8]) -> Result<usize> {
        with_file(fd, |file| {
            let src = &file.value.as_bytes()[file.position..];
            let count = dst.len().min(src.len());
            dst[..count].copy_from_slice(&src[..count]);
            file.position += count;
            Ok(count)
        })
    }

    fn seek(&self, fd: BackendFd, offset: i64, whence: c_int) -> Result<usize> {
        with_file(fd, |file| {
            let len = file.value.as_bytes().len() as u64;
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
const MAX_DEPTH: usize = 3;

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
    match next_name(path) {
        None => Some(Node::Root),
        Some((b"class", rest)) => class(rest),
        Some((b"firmware", rest)) => firmware(rest),
        Some(_) => None,
    }
}

fn class(path: &[u8]) -> Option<Node> {
    let (names, depth) = split(path)?;
    if depth == 0 {
        return Some(Node::Classes);
    }
    let class = *CLASSES.iter().find(|class| class.name() == names[0])?;
    if depth == 1 {
        return Some(Node::Class(class));
    }
    let index = class.find(names[1])?;
    if depth == 2 {
        return Some(Node::Instance(class));
    }
    let attribute = class.attributes().iter().position(|&name| name == names[2])?;
    let mut content = Content::EMPTY;
    class.show(index, attribute, &mut content)?;
    content.push(b"\n");
    Some(Node::File(Value::Text(content)))
}

fn firmware(path: &[u8]) -> Option<Node> {
    let fdt = board().fdt;
    match next_name(path) {
        None => Some(Node::Firmware),
        Some((b"fdt", rest)) if next_name(rest).is_none() => Some(Node::File(Value::Blob(fdt.as_bytes()))),
        Some((b"devicetree", rest)) => match next_name(rest) {
            None => Some(Node::Devicetree),
            Some((b"base", rest)) => devicetree(fdt.root()?, rest),
            Some(_) => None,
        },
        Some(_) => None,
    }
}

fn devicetree(mut node: fdt::Node<'static>, mut path: &[u8]) -> Option<Node> {
    while let Some((name, rest)) = next_name(path) {
        if next_name(rest).is_none()
            && let Some(value) = property(&node, name)
        {
            return Some(Node::File(value));
        }
        node = node.children().find(|child| child.name == name)?;
        path = rest;
    }
    Some(Node::Tree(node))
}

fn property(node: &fdt::Node<'static>, name: &[u8]) -> Option<Value> {
    match node.prop(name) {
        Some(value) => Some(Value::Blob(value)),
        None if name == b"name" => {
            let base = node.name.rsplitn(2, |&c| c == b'@').last().unwrap_or_default();
            let mut content = Content::of(base);
            content.push(b"\0");
            Some(Value::Text(content))
        }
        None => None,
    }
}

fn with_file<R>(fd: BackendFd, f: impl FnOnce(&mut File) -> Result<R>) -> Result<R> {
    FILES.with(|files| f(files.get_mut(fd.0).and_then(Option::as_mut).ok_or(Error::Invalid)?))
}

enum Node {
    Root,
    Classes,
    Class(&'static dyn Class),
    Instance(&'static dyn Class),
    Firmware,
    Devicetree,
    Tree(fdt::Node<'static>),
    File(Value),
}

#[derive(Clone, Copy)]
enum Value {
    Text(Content),
    Blob(&'static [u8]),
}

impl Value {
    fn as_bytes(&self) -> &[u8] {
        match self {
            Value::Text(content) => content.as_bytes(),
            Value::Blob(bytes) => bytes,
        }
    }
}

#[derive(Clone, Copy)]
struct File {
    value: Value,
    position: usize,
}
