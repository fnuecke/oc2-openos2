// SPDX-License-Identifier: MIT

pub mod disk;
pub mod mkfs;
mod volume;

use super::{
    Error, Kind, LABEL_LENGTH, Label, Listing, MNT, Medium, MountInfo, MountText, OpenFlags, Result, Stat,
    is_valid_name,
    vfs::{self, BackendFd, Change, FileSystem, MountId},
};
use crate::arch::KernelCell;
use crate::board::board;
use crate::bytes::until_nul;
use crate::drivers::rtc;
use crate::drivers::virtio::blk::{self, MAX_DISKS, SECTOR_SIZE};
use crate::mem::HeapBox;
use crate::text::Text;
use crate::time::{entropy, fnv1a, mix64};
use core::ffi::c_int;
use disk::{FT_DIR, FT_SYMLINK, ROOT_INODE, S_IFMT, S_IFREG, STATE_CLEAN, STATE_ERROR};
use volume::{Claim, MOUNT_PATH_SIZE, Mount, New, Node};

pub const MAX_FILES: usize = 16;

type MountPath = Text<MOUNT_PATH_SIZE>;

#[derive(Clone, Copy)]
struct File {
    flags: OpenFlags,
    inode: u32,
    pos: u64,
    mount: Option<usize>, // None after medium change
    unlinked: bool,       // a name of the inode was removed while open, see close_file
}

struct State {
    mounts: [Option<HeapBox<Mount>>; MAX_DISKS], // by block device
    root: Option<usize>,
    files: [Option<File>; MAX_FILES],
    generations: u32,
}

static STATE: KernelCell<State> = KernelCell::new(State {
    mounts: [const { None }; MAX_DISKS],
    root: None,
    files: [None; MAX_FILES],
    generations: 0,
});

fn now() -> u32 {
    rtc::seconds() as u32
}

fn is_valid_label(label: &[u8]) -> bool {
    label.len() <= LABEL_LENGTH && is_valid_name(label)
}

fn named_path(name: &[u8]) -> MountPath {
    let mut path = MountPath::of(MNT);
    path.push(name);
    path
}

fn device_path(device: usize) -> MountPath {
    let mut path = MountPath::of(MNT);
    write!(path, "{device}");
    path
}

fn check_mounted(m: Option<&mut Mount>) -> Result<&mut Mount> {
    check_attached(m.filter(|m| !m.parked).ok_or(Error::NotFound)?)
}

fn check_mounted_mut(m: Option<&mut Mount>) -> Result<&mut Mount> {
    check_attached_mut(m.filter(|m| !m.parked).ok_or(Error::NotFound)?)
}

fn check_resolved_mut<'a>(m: Option<&'a mut Mount>, path: &[u8]) -> Result<&'a mut Mount> {
    let m = check_mounted(m)?;
    m.lookup_parent(path)?;
    check_attached_mut(m)
}

fn check_attached(m: &mut Mount) -> Result<&mut Mount> {
    if m.claim == Claim::Writer {
        return Err(Error::Busy);
    }
    match m.medium {
        Medium::Ext2 => Ok(m),
        Medium::Blank => Err(Error::Unformatted),
        _ => Err(Error::Unsupported),
    }
}

fn check_attached_mut(m: &mut Mount) -> Result<&mut Mount> {
    let m = check_attached(m)?;
    if m.claim != Claim::None {
        return Err(Error::Busy);
    }
    if m.writable { Ok(m) } else { Err(Error::ReadOnly) }
}

impl Mount {
    fn leave<T>(&mut self, result: Result<T>) -> Result<T> {
        if matches!(result, Err(Error::Io)) && self.wrote && !self.gone {
            self.sb.state |= STATE_ERROR;
            self.sb.mark_dirty();
        }
        let flushed = self.flush_metadata();
        let value = result?;
        flushed.map(|_| value)
    }

    fn mark_clean(&mut self) -> Result<()> {
        if self.medium != Medium::Ext2 || !self.writable || self.gone {
            return Ok(());
        }
        self.sb.state |= STATE_CLEAN;
        self.sb.mark_dirty();
        self.flush_metadata()
    }

    fn stamp(&mut self) -> Result<()> {
        let previous = fnv1a(0, self.sb.as_bytes());
        self.nonce = mix64(entropy() ^ mix64(previous.wrapping_add(self.device as u64)));
        self.sb.mnt_count = self.sb.mnt_count.wrapping_add(1);
        self.sb.mtime = now();
        self.sb.state &= !STATE_CLEAN;
        self.sb.mark_dirty();
        self.flush_metadata()
    }

    fn read_unusable(&mut self) -> Result<()> {
        self.free_buffers();
        self.writable = !self.dev.read_only();
        let (head, blank) = volume::read_head(self.dev).map_err(|_| Error::Io)?;
        self.disk_head = head;
        if blank {
            self.medium = Medium::Blank;
        }
        Ok(())
    }

    fn park(&mut self) {
        self.free_buffers();
        if !self.parked {
            vfs::unmount(self.path.as_bytes());
        }
        self.parked = true;
        self.path = device_path(self.device);
    }
}

enum Target {
    Replace(volume::Location, Node),
    New(volume::Slot),
}

impl State {
    fn get_mount(&mut self, device: usize) -> Result<&mut Mount> {
        self.mounts[device].as_deref_mut().ok_or(Error::NotFound)
    }

    fn has_open_files(&self, device: usize) -> bool {
        self.files.iter().flatten().any(|f| f.mount == Some(device))
    }

    fn is_open(&self, device: usize, inode: u32) -> bool {
        self.files.iter().flatten().any(|f| f.mount == Some(device) && f.inode == inode)
    }

    fn mark_unlinked(&mut self, device: usize, inode: u32) -> bool {
        let mut open = false;
        for f in self.files.iter_mut().flatten().filter(|f| f.mount == Some(device) && f.inode == inode) {
            f.unlinked = true;
            open = true;
        }
        open
    }

    fn unmount(&mut self, device: usize) {
        for file in self.files.iter_mut().flatten().filter(|f| f.mount == Some(device)) {
            file.mount = None;
        }
        if let Some(m) = self.mounts[device].as_ref().filter(|m| !m.parked) {
            vfs::unmount(m.path.as_bytes());
        }
        self.mounts[device] = None;
        if self.root == Some(device) {
            self.root = None;
        }
    }

    fn is_name_taken(&mut self, name: &[u8], device: usize) -> bool {
        let path = named_path(name);
        let ours = self.get_mount(device).is_ok_and(|m| m.path.as_bytes() == path.as_bytes());
        if vfs::is_mounted(path.as_bytes()) && !ours {
            return true;
        }
        let root = self.root.filter(|&r| r != device).and_then(|root| self.get_mount(root).ok());
        root.is_some_and(|root| root.lookup(&path.as_bytes()[1..]).is_ok())
    }

    fn attach(&mut self, device: usize) {
        let Ok(m) = self.get_mount(device) else { return };
        let label = m.label();
        let label = until_nul(&label);
        let named = m.medium == Medium::Ext2 && is_valid_label(label) && !self.is_name_taken(label, device);
        self.attach_at(device, if named { named_path(label) } else { device_path(device) });
    }

    fn attach_at(&mut self, device: usize, path: MountPath) {
        let Ok(m) = self.get_mount(device) else { return };
        vfs::unmount(m.path.as_bytes());
        if vfs::mount(path.as_bytes(), &Ext2, MountId(device)).is_ok() {
            m.path = path;
        } else {
            m.park();
        }
    }

    fn do_mount(&mut self, device: usize, attach: bool) -> bool {
        let Some(dev) = blk::get(device) else { return false };
        let mut m = loop {
            dev.ack_change();
            // All zeros is a valid Mount.
            let Some(mut m) = (unsafe { HeapBox::<Mount>::zeroed() }) else { return false };
            self.generations = self.generations.wrapping_add(1);
            m.device = device;
            m.generation = self.generations;
            let result = match m.read_mount(dev) {
                Err(Error::Invalid) => m.read_unusable(),
                Ok(()) if m.writable && attach => m.stamp(),
                result => result,
            };
            if m.gone {
                continue; // the medium changed while being read
            }
            if result.is_err() {
                return false;
            }
            break m;
        };
        if !attach {
            m.park();
        }
        self.mounts[device] = Some(m);
        if attach {
            self.attach(device);
        }
        true
    }

    fn make_root(&mut self, device: usize) {
        self.root = Some(device);
        self.attach_at(device, MountPath::of(b"/"));
    }

    fn mount(&mut self, device: usize) {
        if self.do_mount(device, true)
            && device == board().root
            && self.get_mount(device).is_ok_and(|m| m.medium == Medium::Ext2 && !m.parked)
        {
            self.make_root(device);
        }
    }

    fn report(&mut self, old_path: &[u8], device: usize) {
        let new_path = self.mounts[device].as_ref().filter(|m| !m.parked).map(|m| m.path);
        let new_path = new_path.as_ref().map(MountPath::as_bytes);
        if !old_path.is_empty() && new_path == Some(old_path) {
            vfs::medium_event(old_path, Change::Changed);
        } else {
            if !old_path.is_empty() {
                vfs::medium_event(old_path, Change::Removed);
            }
            if let Some(new_path) = new_path {
                vfs::medium_event(new_path, Change::Added);
            }
        }
    }

    fn replace(&mut self, device: usize) {
        let mut old_path = MountPath::EMPTY;
        if let Some(m) = self.mounts[device].as_ref() {
            if !m.parked {
                old_path = m.path;
            }
            self.unmount(device);
        }
        self.mount(device);
        self.report(old_path.as_bytes(), device);
    }

    fn check_media(&mut self) {
        let mut old_paths = [MountPath::EMPTY; MAX_DISKS];
        let mut changed = [false; MAX_DISKS];
        for i in 0..blk::count() {
            let pending = blk::get(i).is_some_and(blk::Disk::change_pending);
            let Some(m) = self.mounts[i].as_deref_mut() else {
                if !pending {
                    continue;
                }
                changed[i] = true;
                continue;
            };
            if !m.gone && !pending {
                continue;
            }
            let may_keep = !m.gone && m.claim != Claim::Writer;
            if may_keep && m.parked {
                if m.is_same_medium() {
                    continue;
                }
            } else if may_keep {
                let dirty = m.is_dirty();
                if m.is_same_medium() {
                    if m.medium != Medium::Ext2 {
                        continue;
                    }
                    m.cache.invalidate();
                    if dirty || m.read_gdt().is_ok() {
                        continue;
                    }
                }
            }
            changed[i] = true;
            if !m.parked {
                old_paths[i] = m.path;
            }
            self.unmount(i);
        }
        for i in 0..blk::count() {
            if changed[i] {
                self.mount(i);
                self.report(old_paths[i].as_bytes(), i);
            }
        }
    }

    fn enter(&mut self, device: usize) -> Option<&mut Mount> {
        let m = self.mounts[device].as_deref_mut()?;
        m.wrote = false;
        Some(m)
    }

    fn enter_device(&mut self, device: usize, generation: u32) -> Result<()> {
        self.check_media();
        let has_medium = blk::get(device).is_some_and(|d| d.sectors() > 0);
        let m = self.mounts[device].as_deref().ok_or(if has_medium { Error::Busy } else { Error::NoMedium })?;
        if generation != 0 && generation != m.generation {
            return Err(Error::Changed);
        }
        Ok(())
    }

    fn open_file(&mut self, device: usize, path: &[u8], flags: OpenFlags) -> Result<BackendFd> {
        let fd = self.files.iter().position(Option::is_none).ok_or(Error::TooMany)?;
        let m = self.get_mount(device)?;
        if path.last() == Some(&b'/') {
            return Err(Error::IsDir);
        }
        let found = m.lookup(path);
        match found {
            Ok(inode) if inode.is_symlink() => return Err(vfs::link_at(path.len())),
            Err(Error::NotFound) if flags.has(OpenFlags::CREATE) => {}
            Err(e) => return Err(e),
            Ok(_) => {}
        }
        if flags.has(OpenFlags::MODIFY) {
            check_attached_mut(m)?;
        }
        let inode = match found {
            Err(Error::NotFound) if flags.has(OpenFlags::CREATE) => m.create(path, New::File),
            Ok(inode) if inode.is_dir() => Err(Error::IsDir),
            Ok(inode) if inode.d.mode & S_IFMT != S_IFREG => Err(Error::Invalid),
            Ok(mut inode) if flags.has(OpenFlags::TRUNCATE) && flags.has(OpenFlags::WRITE) => {
                m.truncate(&mut inode).and_then(|_| {
                    let now = now();
                    inode.d.mtime = now;
                    inode.d.ctime = now;
                    m.write_inode(&inode).map(|_| inode)
                })
            }
            result => result,
        }?;
        self.files[fd] = Some(File { flags, inode: inode.number, pos: 0, mount: Some(device), unlinked: false });
        Ok(BackendFd(fd))
    }

    fn rename_path(&mut self, device: usize, from: &[u8], to: &[u8]) -> Result<()> {
        let m = self.get_mount(device)?;
        let (mut src_parent, src_name) = m.lookup_parent(from)?;
        let src_loc = m.dir_find(&mut src_parent, src_name)?;
        let (mut dst_parent, dst_name) = m.lookup_parent(to)?;
        if !dst_parent.is_dir() {
            return Err(Error::NotDir);
        }
        let mut inode = m.read_inode(src_loc.inode)?;
        let file_type = src_loc.file_type;

        let replaced = match m.dir_find(&mut dst_parent, dst_name) {
            Ok(dst_loc) => {
                if dst_loc.inode == inode.number {
                    return Ok(());
                }
                let replaced = m.read_inode(dst_loc.inode)?;
                if replaced.is_dir() || inode.is_dir() {
                    return Err(Error::Exists);
                }
                Some((dst_loc, replaced))
            }
            Err(Error::NotFound) => None,
            Err(e) => return Err(e),
        };
        let replaced_open = replaced.is_some_and(|(_, replaced)| self.mark_unlinked(device, replaced.number));
        let m = self.get_mount(device)?;

        let moving_dir = inode.is_dir() && src_parent.number != dst_parent.number;
        if moving_dir {
            let mut walk = dst_parent;
            let mut depth = 0u32;
            while walk.number != ROOT_INODE {
                if depth > m.sb.inodes_count {
                    return Err(Error::Io);
                }
                if walk.number == inode.number {
                    return Err(Error::Invalid);
                }
                let up = m.dir_find(&mut walk, b"..").map_err(|_| Error::Io)?;
                walk = m.read_inode(up.inode).map_err(|_| Error::Io)?;
                if !walk.is_dir() {
                    return Err(Error::Io);
                }
                depth += 1;
            }
        }

        let target = match replaced {
            Some((dst_loc, replaced)) => Target::Replace(dst_loc, replaced),
            None => Target::New(m.dir_reserve(&mut dst_parent, dst_name.len())?),
        };
        if moving_dir {
            dst_parent.d.links_count += 1;
            m.write_inode(&dst_parent)?;
        } else if !inode.is_dir() {
            inode.d.links_count += 1;
            m.write_inode(&inode)?;
        }
        match &target {
            Target::Replace(dst_loc, _) => m.dir_set_entry(&mut dst_parent, dst_loc, inode.number, file_type)?,
            Target::New(slot) => m.dir_put(&mut dst_parent, *slot, dst_name, inode.number, file_type)?,
        }
        if src_parent.number == dst_parent.number {
            src_parent = dst_parent;
        }
        let src_loc = m.dir_find(&mut src_parent, src_name)?;
        m.dir_remove(&mut src_parent, &src_loc)?;
        if moving_dir {
            m.dir_set_parent(&mut inode, dst_parent.number)?;
            src_parent.d.links_count -= 1;
            m.write_inode(&src_parent)?;
        } else if !inode.is_dir() {
            inode.d.links_count -= 1;
            m.write_inode(&inode)?;
        }
        if let Target::Replace(_, mut replaced) = target {
            m.unlink_inode(&mut replaced, replaced_open);
        }
        Ok(())
    }

    fn remove_path(&mut self, device: usize, path: &[u8]) -> Result<()> {
        let m = self.get_mount(device)?;
        let (mut parent, name) = m.lookup_parent(path)?;
        let loc = m.dir_find(&mut parent, name)?;
        let mut inode = m.read_inode(loc.inode)?;
        let open = self.mark_unlinked(device, inode.number);
        let m = self.get_mount(device)?;
        if inode.is_dir() && !m.is_dir_empty(&mut inode) {
            return Err(Error::NotEmpty);
        }
        m.dir_remove(&mut parent, &loc)?;
        if inode.is_dir() {
            parent.d.links_count -= 1;
            let _ = m.write_inode(&parent);
            inode.d.links_count = 0;
            inode.d.dtime = now();
            if m.truncate(&mut inode).is_ok() {
                m.free_inode(&mut inode);
            }
        } else {
            m.unlink_inode(&mut inode, open);
        }
        Ok(())
    }

    fn close_file(&mut self, fd: usize) -> Result<()> {
        let file = self.files.get_mut(fd).and_then(Option::take).ok_or(Error::Invalid)?;
        let last = |device: usize| file.unlinked && !self.is_open(device, file.inode);
        let Some(device) = file.mount.filter(|&device| last(device)) else { return Ok(()) };
        let Ok(m) = check_mounted_mut(self.enter(device)) else { return Ok(()) };
        let result = m.read_inode(file.inode).map(|mut inode| {
            if inode.d.links_count == 0 {
                m.free_unlinked(&mut inode);
            }
        });
        let _ = m.leave(result);
        Ok(())
    }

    fn enter_file(&mut self, fd: BackendFd) -> Result<(&mut Mount, &mut File)> {
        self.check_media();
        let file = self.files.get_mut(fd.0).and_then(Option::as_mut).ok_or(Error::Invalid)?;
        let device = file.mount.ok_or(Error::Changed)?;
        let m = self.mounts[device].as_deref_mut().ok_or(Error::Changed)?;
        m.wrote = false;
        Ok((m, file))
    }
}

// ---------------------------------------------------------------------------------------- //
// Mounts and media

pub fn mount_all() {
    STATE.with(|s| {
        for i in 0..blk::count() {
            s.mount(i);
        }
    });
}

pub fn unmount_all() {
    STATE.with(|s| {
        for m in s.mounts[..blk::count()].iter_mut().flatten() {
            m.claim.end_reads();
        }
        for fd in 0..MAX_FILES {
            let _ = s.close_file(fd);
        }
        for m in s.mounts[..blk::count()].iter_mut().flatten() {
            if !m.parked && m.claim != Claim::Writer {
                let _ = m.mark_clean();
            }
        }
    });
}

pub fn poll_media() {
    STATE.with(State::check_media);
}

pub fn unmount(device: usize, generation: u32) -> Result<()> {
    STATE.with(|s| {
        s.enter_device(device, generation)?;
        if Some(device) == s.root {
            return Err(Error::Busy);
        }
        let busy = s.has_open_files(device);
        let m = s.get_mount(device)?;
        if m.parked {
            return Err(Error::Invalid);
        }
        if busy || m.claim != Claim::None {
            return Err(Error::Busy);
        }
        m.mark_clean()?;
        let old_path = m.path;
        m.park();
        s.report(old_path.as_bytes(), device);
        Ok(())
    })
}

pub fn mount(device: usize) -> Result<()> {
    STATE.with(|s| {
        s.enter_device(device, 0)?;
        let m = s.get_mount(device)?;
        if !m.parked || m.claim != Claim::None {
            return Err(Error::Busy);
        }
        s.unmount(device);
        s.mount(device);
        s.report(b"", device);
        match s.mounts[device].as_ref() {
            Some(m) if m.parked => Err(Error::Exists),
            Some(_) => Ok(()),
            None => Err(Error::Io),
        }
    })
}

pub fn mount_info(device: usize) -> Option<MountInfo> {
    STATE.with(|s| {
        let m = s.mounts[device].as_deref()?;
        Some(MountInfo {
            path: MountText::of(m.path.as_bytes()),
            label: MountText::of(until_nul(&m.label())),
            index: device,
            medium: m.medium,
            generation: m.generation,
            total: if m.medium == Medium::Ext2 { m.total() } else { m.dev.sectors() * SECTOR_SIZE as u64 },
            free: if m.medium == Medium::Ext2 { m.free() } else { 0 },
            writable: m.writable,
            unmounted: m.parked,
        })
    })
}

// ---------------------------------------------------------------------------------------- //
// The file system

pub struct Ext2;

impl FileSystem for Ext2 {
    fn stat(&self, device: MountId, path: &[u8]) -> Result<Stat> {
        STATE.with(|s| {
            let inode = check_mounted(s.enter(device.0))?.lookup(path)?;
            Ok(Stat { size: inode.d.size as u64, mtime: inode.d.mtime, kind: inode.kind() })
        })
    }

    fn readlink(&self, device: MountId, path: &[u8], out: &mut [u8]) -> Result<usize> {
        STATE.with(|s| {
            let m = check_mounted(s.enter(device.0))?;
            let mut inode = m.lookup(path)?;
            if !inode.is_symlink() {
                return Err(Error::Invalid);
            }
            m.read_link(&mut inode, out)
        })
    }

    fn list(&self, device: MountId, path: &[u8], out: &mut Listing) -> Result<()> {
        STATE.with(|s| {
            let m = check_mounted(s.enter(device.0))?;
            let mut dir = m.lookup(path)?;
            if dir.is_symlink() {
                return Err(vfs::link_at(path.len()));
            } else if !dir.is_dir() {
                return Err(Error::NotDir);
            }
            let filetype = m.has_filetype();
            m.dir_entries(&mut dir, |m, name, inode, file_type| {
                let kind = match file_type {
                    _ if !filetype => m.read_inode(inode).map_or(Kind::File, |n| n.kind()),
                    FT_DIR => Kind::Directory,
                    FT_SYMLINK => Kind::Link,
                    _ => Kind::File,
                };
                out.add(name, kind).map_err(|_| Error::NoSpace)
            })
        })
    }

    fn open(&self, device: MountId, path: &[u8], flags: OpenFlags) -> Result<BackendFd> {
        STATE.with(|s| {
            let device = check_mounted(s.enter(device.0))?.device;
            let fd = s.open_file(device, path, flags);
            let _ = s.get_mount(device).and_then(|m| m.leave(fd.map(|_| ())));
            fd
        })
    }

    fn read(&self, fd: BackendFd, dst: &mut [u8]) -> Result<usize> {
        STATE.with(|s| {
            let (m, file) = s.enter_file(fd)?;
            if !file.flags.has(OpenFlags::READ) {
                return Err(Error::BadFd);
            }
            let mut inode = m.read_inode(file.inode)?;
            if file.pos >= inode.d.size as u64 {
                return Ok(0);
            }
            let len = (dst.len() as u64).min(inode.d.size as u64 - file.pos) as usize;
            m.read_file(&mut inode, &mut file.pos, &mut dst[..len])
        })
    }

    fn write(&self, fd: BackendFd, src: &[u8]) -> Result<usize> {
        STATE.with(|s| {
            let (m, file) = s.enter_file(fd)?;
            if !file.flags.has(OpenFlags::WRITE) {
                return Err(Error::BadFd);
            }
            let result = 'write: {
                let mut inode = match m.read_inode(file.inode) {
                    Ok(inode) => inode,
                    Err(e) => break 'write Err(e),
                };
                if file.flags.has(OpenFlags::APPEND) {
                    file.pos = inode.d.size as u64;
                }
                if file.pos + src.len() as u64 > u32::MAX as u64 {
                    break 'write Err(Error::NoSpace);
                }
                match m.write_file(&mut inode, &mut file.pos, src) {
                    (0, result) => result.map(|_| 0),
                    (done, _) => Ok(done),
                }
            };
            m.leave(result)
        })
    }

    fn seek(&self, fd: BackendFd, offset: i64, whence: c_int) -> Result<usize> {
        STATE.with(|s| {
            let (m, file) = s.enter_file(fd)?;
            file.pos = super::seek(file.pos, offset, whence, || Ok(m.read_inode(file.inode)?.d.size as u64))?;
            Ok(file.pos as usize)
        })
    }

    fn close(&self, fd: BackendFd) -> Result<()> {
        STATE.with(|s| s.close_file(fd.0))
    }

    fn mkdir(&self, device: MountId, path: &[u8]) -> Result<()> {
        STATE.with(|s| {
            let m = check_resolved_mut(s.enter(device.0), path)?;
            let result = m.create(path, New::Dir).map(|_| ());
            m.leave(result)
        })
    }

    fn symlink(&self, device: MountId, path: &[u8], target: &[u8]) -> Result<()> {
        STATE.with(|s| {
            let m = check_resolved_mut(s.enter(device.0), path)?;
            let result = m.create(path, New::Link(target)).map(|_| ());
            m.leave(result)
        })
    }

    fn link(&self, device: MountId, old: &[u8], new: &[u8]) -> Result<()> {
        STATE.with(|s| {
            let m = check_mounted_mut(s.enter(device.0))?;
            let result = m.link(old, new);
            m.leave(result)
        })
    }

    fn rename(&self, device: MountId, from: &[u8], to: &[u8]) -> Result<()> {
        STATE.with(|s| {
            let device = check_mounted_mut(s.enter(device.0))?.device;
            let result = s.rename_path(device, from, to);
            s.get_mount(device)?.leave(result)
        })
    }

    fn remove(&self, device: MountId, path: &[u8]) -> Result<()> {
        STATE.with(|s| {
            let device = check_resolved_mut(s.enter(device.0), path)?.device;
            let result = s.remove_path(device, path);
            s.get_mount(device)?.leave(result)
        })
    }

    fn info(&self, device: MountId) -> Result<(u64, u64)> {
        STATE.with(|s| {
            let m = check_mounted(s.enter(device.0))?;
            Ok((m.total(), m.free()))
        })
    }
}

// ---------------------------------------------------------------------------------------- //
// Drives

pub fn claim(device: usize, write: bool) -> Result<u32> {
    STATE.with(|s| {
        s.enter_device(device, 0)?;
        if write {
            if Some(device) == s.root {
                return Err(Error::Busy);
            }
            let busy = s.has_open_files(device);
            let m = s.get_mount(device)?;
            if m.dev.read_only() {
                return Err(Error::ReadOnly);
            }
            if busy {
                return Err(Error::Busy);
            }
            m.claim.try_write()?;
        } else {
            if s.files.iter().flatten().any(|f| f.mount == Some(device) && f.flags.has(OpenFlags::WRITE)) {
                return Err(Error::Busy);
            }
            s.get_mount(device)?.claim.try_read()?;
        }
        Ok(s.get_mount(device)?.generation)
    })
}

pub fn release(device: usize, write: bool, generation: u32) {
    STATE.with(|s| {
        s.check_media();
        let Some(m) = s.mounts[device].as_deref_mut().filter(|m| m.generation == generation) else { return };
        if !write {
            m.claim.end_read();
        } else if m.parked {
            s.unmount(device);
            s.do_mount(device, false);
        } else {
            s.replace(device);
        }
    })
}

pub fn generation(device: usize) -> Option<u32> {
    STATE.with(|s| s.mounts[device].as_ref().filter(|m| !m.gone).map(|m| m.generation))
}

impl mkfs::Medium for blk::Disk {
    fn sectors(&self) -> u64 {
        blk::Disk::sectors(*self)
    }

    fn read_only(&self) -> bool {
        blk::Disk::read_only(*self)
    }

    fn write(&mut self, sector: u64, data: &[u8]) -> Result<()> {
        blk::Disk::write(*self, sector, data)
            .map_err(|f| if f == blk::Fault::Changed { Error::Changed } else { Error::Io })
    }
}

pub fn format(device: usize, label: Option<&[u8]>, generation: u32) -> Result<()> {
    STATE.with(|s| {
        s.enter_device(device, generation)?;
        if Some(device) == s.root {
            return Err(Error::Busy);
        }
        let current = s.get_mount(device)?.label();
        let current = until_nul(&current);
        let label = label.unwrap_or(if is_valid_label(current) { current } else { b"" });
        if !label.is_empty() && !is_valid_label(label) {
            return Err(Error::Invalid);
        }
        if !label.is_empty() && s.is_name_taken(label, device) {
            return Err(Error::Exists);
        }
        let busy = s.has_open_files(device);
        let m = s.get_mount(device)?;
        if m.dev.read_only() {
            return Err(Error::ReadOnly);
        }
        if busy || m.claim != Claim::None {
            return Err(Error::Busy);
        }
        let mut dev = m.dev;
        let result = mkfs::format(&mut dev, label, now(), |i| mix64(entropy().wrapping_add(i)));
        s.replace(device);
        result
    })
}

pub fn set_label(device: usize, label: &[u8], generation: u32) -> Result<()> {
    STATE.with(|s| {
        s.enter_device(device, generation)?;
        if !label.is_empty() && !is_valid_label(label) {
            return Err(Error::Invalid);
        }
        let parked = check_attached_mut(s.get_mount(device)?)?.parked;
        let old_path = s.get_mount(device)?.path;
        let is_root = Some(device) == s.root;
        if !is_root {
            if !label.is_empty() && s.is_name_taken(label, device) {
                return Err(Error::Exists);
            }
            if s.has_open_files(device) {
                return Err(Error::Busy);
            }
        }
        let m = s.get_mount(device)?;
        m.sb.volume_name = [0; LABEL_LENGTH];
        m.sb.volume_name[..label.len()].copy_from_slice(label);
        m.sb.mark_dirty();
        if parked {
            let result = m.flush_super();
            if result.is_err() {
                m.sb.discard();
            }
            return result;
        }
        m.leave(Ok(()))?;
        if is_root {
            return Ok(());
        }
        s.attach(device);
        if s.get_mount(device)?.path.as_bytes() != old_path.as_bytes() {
            s.report(old_path.as_bytes(), device);
        }
        Ok(())
    })
}

pub fn label(device: usize) -> Result<Label> {
    STATE.with(|s| {
        s.enter_device(device, 0)?;
        Ok(Label::of(until_nul(&check_attached(s.get_mount(device)?)?.label())))
    })
}
