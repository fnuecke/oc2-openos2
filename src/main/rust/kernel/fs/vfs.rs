// SPDX-License-Identifier: MIT

use super::devfs::{self, Poll};
use super::{
    Error, Kind, Label, Listing, Medium, MountInfo, MountText, OpenFlags, PATH_LENGTH, Result, Stat, ext2, next_name,
    p9, sysfs, tmpfs, trim_end,
};
use crate::arch::KernelCell;
use crate::drivers::virtio::blk::{self, MAX_DISKS};
use crate::power;
use crate::signal::{self, Event, SOURCES};
use core::ffi::c_int;

register!(SOURCES, &Media);

pub trait FileSystem: Sync {
    fn stat(&self, mount: MountId, path: &[u8]) -> Result<Stat>;

    fn readlink(&self, _mount: MountId, _path: &[u8], _out: &mut [u8]) -> Result<usize> {
        Err(Error::Invalid)
    }

    fn list(&self, mount: MountId, path: &[u8], out: &mut Listing) -> Result<()>;

    fn open(&self, mount: MountId, path: &[u8], flags: OpenFlags) -> Result<BackendFd>;

    fn read(&self, fd: BackendFd, dst: &mut [u8]) -> Result<usize>;

    fn write(&self, _fd: BackendFd, _src: &[u8]) -> Result<usize> {
        Err(Error::ReadOnly)
    }

    fn seek(&self, fd: BackendFd, offset: i64, whence: c_int) -> Result<usize>;

    fn poll(&self, _fd: BackendFd) -> Result<Poll> {
        Ok(Poll { readable: true, writable: true })
    }

    fn close(&self, fd: BackendFd) -> Result<()>;

    fn mkdir(&self, _mount: MountId, _path: &[u8]) -> Result<()> {
        Err(Error::ReadOnly)
    }

    fn link(&self, _mount: MountId, _old: &[u8], _new: &[u8]) -> Result<()> {
        Err(Error::NotPermitted)
    }

    fn rename(&self, _mount: MountId, _from: &[u8], _to: &[u8]) -> Result<()> {
        Err(Error::ReadOnly)
    }

    fn remove(&self, _mount: MountId, _path: &[u8]) -> Result<()> {
        Err(Error::ReadOnly)
    }

    fn symlink(&self, _mount: MountId, _path: &[u8], _target: &[u8]) -> Result<()> {
        Err(Error::NotPermitted)
    }

    fn info(&self, _mount: MountId) -> Result<(u64, u64)> {
        Err(Error::Invalid)
    }
}

#[derive(Clone, Copy)]
pub enum Change {
    Added,
    Removed,
    Changed,
}

struct Media;

impl signal::Source for Media {
    fn pending(&self) -> bool {
        MEDIUM_EVENTS.with(|q| q.head != q.tail)
            || (0..blk::count()).filter_map(blk::get).any(blk::Disk::change_pending)
    }

    fn take(&self) -> Option<Event> {
        let event = MEDIUM_EVENTS.with(|q| {
            if q.head == q.tail {
                return None;
            }
            let event = q.events[(q.tail % MAX_MEDIUM_EVENTS as u32) as usize];
            q.tail = q.tail.wrapping_add(1);
            Some(event)
        })?;
        let detail = match event.what {
            Change::Added => c"added",
            Change::Removed => c"removed",
            Change::Changed => c"changed",
        };
        Some(Event { detail: Some(detail), ..Event::with_value(c"medium", event.path.as_bytes()) })
    }

    fn collect(&self) {
        poll_media();
    }
}

pub struct Path {
    data: [u8; PATH_LENGTH],
    len: usize,
}

impl Path {
    pub fn as_bytes(&self) -> &[u8] {
        &self.data[..self.len]
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct MountId(pub usize);

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BackendFd(pub usize);

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Fd(pub usize);

const MAX_MOUNTS: usize = 4 + MAX_DISKS + p9::MAX_MOUNTS; // /, /dev, /sys, /tmp, ...
const MAX_FILES: usize = ext2::MAX_FILES + p9::MAX_FILES + sysfs::MAX_FILES + devfs::MAX_FILES + tmpfs::MAX_FILES;
const MAX_MEDIUM_EVENTS: usize = 16;
const MAX_LINKS: usize = 40;

static MOUNTS: KernelCell<[Option<Mount>; MAX_MOUNTS]> = KernelCell::new([None; MAX_MOUNTS]);
static FILES: KernelCell<[Option<File>; MAX_FILES]> = KernelCell::new([None; MAX_FILES]);
static MEDIUM_EVENTS: KernelCell<MediumEvents> =
    KernelCell::new(MediumEvents { events: [MediumEvent::EMPTY; MAX_MEDIUM_EVENTS], head: 0, tail: 0 });
static LINK: KernelCell<Option<usize>> = KernelCell::new(None);

pub fn mount_all() {
    mount(b"/dev", &devfs::Devfs, MountId(0)).expect("the table has room");
    mount(b"/sys", &sysfs::Sysfs, MountId(0)).expect("the table has room");
    tmpfs::mount();
    ext2::mount_all();
}

pub fn unmount_all() {
    ext2::unmount_all();
}

pub fn poll_media() {
    ext2::poll_media();
}

pub fn mount(path: &[u8], fs: &'static dyn FileSystem, instance: MountId) -> Result<()> {
    MOUNTS.with(|mounts| {
        if mounts.iter().flatten().any(|m| m.path.as_bytes() == path) {
            return Err(Error::Exists);
        }
        let free = mounts.iter_mut().find(|m| m.is_none()).ok_or(Error::TooMany)?;
        *free = Some(Mount { path: MountText::of(path), fs, instance });
        Ok(())
    })
}

pub fn unmount(path: &[u8]) {
    MOUNTS.with(|mounts| {
        for m in mounts.iter_mut().filter(|m| m.as_ref().is_some_and(|m| m.path.as_bytes() == path)) {
            *m = None;
        }
    });
}

pub fn is_mounted(path: &[u8]) -> bool {
    MOUNTS.with(|mounts| mounts.iter().flatten().any(|m| m.path.as_bytes() == path))
}

pub fn medium_event(path: &[u8], what: Change) {
    MEDIUM_EVENTS.with(|q| {
        if q.head.wrapping_sub(q.tail) == MAX_MEDIUM_EVENTS as u32 {
            return;
        }
        q.events[(q.head % MAX_MEDIUM_EVENTS as u32) as usize] = MediumEvent { path: MountText::of(path), what };
        q.head = q.head.wrapping_add(1);
    });
}

pub fn normalize(path: &[u8]) -> Result<Path> {
    let mut out = Path { data: [0; PATH_LENGTH], len: 1 };
    out.data[0] = b'/';
    let mut rest = path;
    while let Some((name, after)) = next_name(rest) {
        rest = after;
        match name {
            b"." => {}
            b".." => {
                while out.len > 1 && out.data[out.len - 1] != b'/' {
                    out.len -= 1;
                }
                if out.len > 1 {
                    out.len -= 1;
                }
            }
            _ => {
                if out.len + name.len() + 2 >= PATH_LENGTH {
                    return Err(Error::NameTooLong);
                }
                if out.len > 1 {
                    out.data[out.len] = b'/';
                    out.len += 1;
                }
                out.data[out.len..out.len + name.len()].copy_from_slice(name);
                out.len += name.len();
            }
        }
    }
    if out.len > 1 && path.last() == Some(&b'/') {
        out.data[out.len] = b'/';
        out.len += 1;
    }
    out.data[out.len] = 0;
    Ok(out)
}

pub fn mount_count() -> usize {
    ext2::poll_media();
    blk::count() + 2 + p9::mount_count() + 1
}

pub fn mount_info(index: usize) -> Option<MountInfo> {
    if index < blk::count() {
        return ext2::mount_info(index);
    }
    match index - blk::count() {
        0 => Some(pseudo_mount_info(b"/dev", b"devfs", Medium::Devfs, true)),
        1 => Some(pseudo_mount_info(b"/sys", b"sysfs", Medium::Sysfs, false)),
        index if index - 2 < p9::mount_count() => Some(p9::mount_info(index - 2)),
        index if index - 2 == p9::mount_count() => Some(tmpfs::mount_info()),
        _ => None,
    }
}

pub fn unmount_drive(path: &[u8], generation: u32) -> Result<()> {
    ext2::unmount(disk(path)?, generation)
}

pub fn mount_drive(path: &[u8]) -> Result<()> {
    ext2::mount(disk(path)?)
}

pub fn info(path: &[u8]) -> Result<(u64, u64)> {
    let mut path = enter(path)?;
    stat_path(&mut path, true)?;
    let (m, _) = resolve(path.as_bytes())?;
    m.fs.info(m.instance)
}

pub fn stat(path: &[u8]) -> Result<Stat> {
    stat_path(&mut enter(path)?, true)
}

pub fn lstat(path: &[u8]) -> Result<Stat> {
    stat_path(&mut enter(path)?, false)
}

pub fn readlink(path: &[u8], out: &mut [u8]) -> Result<usize> {
    walk(&mut enter(path)?, |m, rest| m.fs.readlink(m.instance, rest, out))
}

pub fn link_at(at: usize) -> Error {
    LINK.with(|link| *link = Some(at));
    Error::Loop
}

pub fn list(path: &[u8], out: &mut Listing) -> Result<()> {
    let mut path = enter(path)?;
    let result = walk(&mut path, |m, rest| m.fs.list(m.instance, rest, out));
    if result.is_err_and(|e| e != Error::NotFound) {
        return result;
    }
    let dir = trim_end(path.as_bytes());
    let mut found = false;
    MOUNTS.with(|mounts| {
        for name in mounts.iter().flatten().filter_map(|m| under(m.path.as_bytes(), dir).and_then(next_name)) {
            let _ = out.add_unique(name.0, Kind::Directory);
            found = true;
        }
    });
    if found { Ok(()) } else { result }
}

pub fn label(path: &[u8]) -> Result<Label> {
    ext2::label(disk(path)?)
}

pub fn open(path: &[u8], flags: OpenFlags) -> Result<Fd> {
    let mut path = enter(path)?;
    let fd = take_file(resolve(path.as_bytes())?.0.fs)?;
    let opened = walk(&mut path, |m, rest| m.fs.open(m.instance, rest, flags).map(|id| File { fs: m.fs, id }));
    put_file(fd, opened)
}

pub fn mkdir(path: &[u8]) -> Result<()> {
    let mut path = enter(path)?;
    refuse_link_slash(&path, Error::Exists)?;
    walk(&mut path, |m, rest| m.fs.mkdir(m.instance, rest))
}

pub fn open_anonymous() -> Result<Fd> {
    power::check();
    let fd = take_file(&tmpfs::Tmpfs)?;
    put_file(fd, tmpfs::open_anonymous().map(|id| File { fs: &tmpfs::Tmpfs, id }))
}

pub fn link(old: &[u8], new: &[u8]) -> Result<()> {
    let (mut old, mut new) = (enter(old)?, enter(new)?);
    refuse_link_slash(&old, Error::NotDir)?;
    refuse_link_slash(&new, Error::Exists)?;
    walk_parent(&mut old)?;
    walk_parent(&mut new)?;
    let ((m, old), (target, new)) = (resolve(old.as_bytes())?, resolve(new.as_bytes())?);
    if m.slot != target.slot {
        return Err(Error::CrossDevice);
    }
    m.fs.link(m.instance, old, new)
}

pub fn rename(from: &[u8], to: &[u8]) -> Result<()> {
    let (mut from, mut to) = (enter(from)?, enter(to)?);
    refuse_link_slash(&from, Error::NotDir)?;
    refuse_link_slash(&to, Error::NotDir)?;
    walk_parent(&mut from)?;
    walk_parent(&mut to)?;
    let ((m, from), (target, to)) = (resolve(from.as_bytes())?, resolve(to.as_bytes())?);
    if m.slot != target.slot {
        return Err(Error::CrossDevice);
    }
    m.fs.rename(m.instance, from, to)
}

pub fn remove(path: &[u8]) -> Result<()> {
    let mut path = enter(path)?;
    refuse_link_slash(&path, Error::NotDir)?;
    walk(&mut path, |m, rest| m.fs.remove(m.instance, rest))
}

pub fn realpath(path: &[u8], out: &mut [u8; PATH_LENGTH]) -> Result<usize> {
    let mut path = enter(path)?;
    stat_path(&mut path, true)?;
    let real = trim_end(path.as_bytes());
    out[..real.len()].copy_from_slice(real);
    Ok(real.len())
}

pub fn symlink(target: &[u8], path: &[u8]) -> Result<()> {
    if target.is_empty() {
        return Err(Error::NotFound);
    }
    if target.len() >= PATH_LENGTH {
        return Err(Error::NameTooLong);
    }
    let mut path = enter(path)?;
    refuse_link_slash(&path, Error::Exists)?;
    walk(&mut path, |m, rest| m.fs.symlink(m.instance, rest, target))
}

pub fn format(path: &[u8], label: Option<&[u8]>, generation: u32) -> Result<()> {
    ext2::format(disk(path)?, label, generation)
}

pub fn set_label(path: &[u8], label: &[u8], generation: u32) -> Result<()> {
    ext2::set_label(disk(path)?, label, generation)
}

pub fn read(fd: Fd, dst: &mut [u8]) -> Result<usize> {
    power::check();
    let file = file(fd)?;
    file.fs.read(file.id, dst)
}

pub fn write(fd: Fd, src: &[u8]) -> Result<usize> {
    power::check();
    let file = file(fd)?;
    file.fs.write(file.id, src)
}

pub fn seek(fd: Fd, offset: i64, whence: c_int) -> Result<usize> {
    power::check();
    let file = file(fd)?;
    file.fs.seek(file.id, offset, whence)
}

pub fn poll(fd: Fd) -> Result<Poll> {
    let file = file(fd)?;
    file.fs.poll(file.id)
}

pub fn close(fd: Fd) -> Result<()> {
    power::check();
    let file = FILES.with(|files| files.get_mut(fd.0).and_then(Option::take)).ok_or(Error::BadFd)?;
    file.fs.close(file.id)
}

fn enter(path: &[u8]) -> Result<Path> {
    power::check();
    poll_media();
    normalize(path)
}

fn disk(path: &[u8]) -> Result<usize> {
    let path = enter(path)?;
    under(path.as_bytes(), b"/dev").ok_or(Error::NotBlock).and_then(devfs::disk)
}

fn pseudo_mount_info(path: &[u8], label: &[u8], medium: Medium, writable: bool) -> MountInfo {
    MountInfo {
        path: MountText::of(path),
        label: MountText::of(label),
        index: 0,
        medium,
        generation: 0,
        total: 0,
        free: 0,
        writable,
        unmounted: false,
    }
}

fn stat_path(path: &mut Path, follow: bool) -> Result<Stat> {
    let result = walk(path, |m, rest| match m.fs.stat(m.instance, rest)? {
        stat if follow && stat.kind == Kind::Link => Err(link_at(rest.len())),
        stat => Ok(stat),
    });
    match result {
        Err(Error::NotFound) if has_mounts_in(path.as_bytes()) => Ok(Stat { kind: Kind::Directory, ..Stat::default() }),
        result => result,
    }
}

fn walk<T>(path: &mut Path, mut f: impl FnMut(Mounted, &[u8]) -> Result<T>) -> Result<T> {
    let mut links = 0;
    loop {
        let (m, rest) = resolve(path.as_bytes())?;
        let start = path.len - rest.len();
        LINK.with(|link| *link = None);
        match (f(m, rest), LINK.with(Option::take)) {
            (Err(Error::Loop), Some(at)) if links < MAX_LINKS => substitute(path, m, start, start + at)?,
            (result, _) => return result,
        }
        links += 1;
    }
}

fn refuse_link_slash(path: &Path, error: Error) -> Result<()> {
    if path.as_bytes().len() > 1 && path.as_bytes().ends_with(b"/") {
        let stat = stat_path(&mut normalize(trim_end(path.as_bytes()))?, false);
        if stat.is_ok_and(|stat| stat.kind == Kind::Link) {
            return Err(error);
        }
    }
    Ok(())
}

fn substitute(path: &mut Path, m: Mounted, start: usize, end: usize) -> Result<()> {
    let mut target = [0; PATH_LENGTH];
    let len = m.fs.readlink(m.instance, &path.as_bytes()[start..end], &mut target)?;
    if len == 0 {
        return Err(Error::NotFound);
    }
    let link = &path.as_bytes()[..end];
    let base = if target[0] == b'/' { 0 } else { link.iter().rposition(|&c| c == b'/').unwrap_or(0) + 1 };
    let rest = &path.as_bytes()[end..];
    if base + len + rest.len() >= PATH_LENGTH {
        return Err(Error::NameTooLong);
    }
    let mut joined = [0; PATH_LENGTH];
    joined[..base].copy_from_slice(&path.as_bytes()[..base]);
    joined[base..base + len].copy_from_slice(&target[..len]);
    joined[base + len..base + len + rest.len()].copy_from_slice(rest);
    *path = normalize(&joined[..base + len + rest.len()])?;
    Ok(())
}

fn walk_parent(path: &mut Path) -> Result<()> {
    walk(path, |m, rest| {
        let rest = trim_end(rest);
        let dir = &rest[..rest.iter().rposition(|&c| c == b'/').unwrap_or(0)];
        match m.fs.stat(m.instance, dir)? {
            stat if stat.kind == Kind::Link => Err(link_at(dir.len())),
            _ => Ok(()),
        }
    })
}

fn resolve(path: &[u8]) -> Result<(Mounted, &[u8])> {
    MOUNTS.with(|mounts| {
        let mut found: Option<(Mounted, &[u8], usize)> = None;
        for (slot, m) in mounts.iter().enumerate() {
            let Some(m) = m else { continue };
            let Some(rest) = under(path, m.path.as_bytes()) else { continue };
            if found.is_none_or(|(_, _, len)| m.path.len() > len) {
                found = Some((Mounted { slot, fs: m.fs, instance: m.instance }, rest, m.path.len()));
            }
        }
        found.map(|(m, rest, _)| (m, rest)).ok_or(Error::NotFound)
    })
}

fn under<'a>(path: &'a [u8], dir: &[u8]) -> Option<&'a [u8]> {
    if dir == b"/" {
        return path.strip_prefix(b"/");
    }
    path.strip_prefix(dir).filter(|rest| rest.is_empty() || rest[0] == b'/')
}

fn has_mounts_in(path: &[u8]) -> bool {
    let dir = trim_end(path);
    MOUNTS.with(|mounts| mounts.iter().flatten().any(|m| under(m.path.as_bytes(), dir).is_some_and(|r| !r.is_empty())))
}

fn take_file(fs: &'static dyn FileSystem) -> Result<Fd> {
    FILES.with(|files| {
        let index = files.iter().position(Option::is_none).ok_or(Error::TooMany)?;
        files[index] = Some(File { fs, id: BackendFd(usize::MAX) });
        Ok(Fd(index))
    })
}

fn put_file(fd: Fd, opened: Result<File>) -> Result<Fd> {
    FILES.with(|files| files[fd.0] = opened.as_ref().ok().copied());
    opened.map(|_| fd)
}

fn file(fd: Fd) -> Result<File> {
    FILES.with(|files| files.get(fd.0).copied().flatten()).ok_or(Error::BadFd)
}

#[derive(Clone, Copy)]
struct Mount {
    path: MountText,
    fs: &'static dyn FileSystem,
    instance: MountId,
}

#[derive(Clone, Copy)]
struct Mounted {
    slot: usize,
    fs: &'static dyn FileSystem,
    instance: MountId,
}

#[derive(Clone, Copy)]
struct File {
    fs: &'static dyn FileSystem,
    id: BackendFd,
}

#[derive(Clone, Copy)]
struct MediumEvent {
    path: MountText,
    what: Change,
}

impl MediumEvent {
    const EMPTY: Self = Self { path: MountText::EMPTY, what: Change::Added };
}

struct MediumEvents {
    events: [MediumEvent; MAX_MEDIUM_EVENTS],
    head: u32,
    tail: u32,
}
