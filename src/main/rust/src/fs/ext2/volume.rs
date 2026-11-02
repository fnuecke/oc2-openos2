// SPDX-License-Identifier: MIT

use super::disk::*;
use crate::api::kprintf;
use crate::bytes::le32;
use crate::drivers::rtc;
use crate::drivers::virtio::blk::{Disk, Fault, SECTOR_SIZE};
use crate::fs::{Data, Error, Kind, LABEL_LENGTH, MNT, Medium, Result, vfs};
use crate::mem::HeapBuf;
use crate::power::halt;
use crate::text::Text;
use crate::time::{FNV_OFFSET, fnv1a};
use core::ops::{Deref, DerefMut};

pub const MOUNT_PATH_SIZE: usize = MNT.len() + LABEL_LENGTH + 1;
const FLOPPY_FILL: u8 = 0xE5;
const INODE_INDEX_FL: u32 = 0x1000;
const CACHE_SIZE: usize = 12;

#[derive(Clone, Copy, Default)]
pub struct Node {
    pub number: u32,
    pub d: Inode,
}

impl Node {
    pub fn is_dir(&self) -> bool {
        self.d.mode & S_IFMT == S_IFDIR
    }

    pub fn is_symlink(&self) -> bool {
        self.d.mode & S_IFMT == S_IFLNK
    }

    pub fn kind(&self) -> Kind {
        match self.d.mode & S_IFMT {
            S_IFDIR => Kind::Directory,
            S_IFLNK => Kind::Link,
            _ => Kind::File,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Location {
    block_index: u32,
    offset: usize,
    prev_offset: Option<usize>,
    pub inode: u32,
    pub file_type: u8,
}

#[derive(Clone, Copy)]
pub enum New<'a> {
    File,
    Dir,
    Link(&'a [u8]),
}

#[derive(Clone, Copy, Default)]
pub struct Slot {
    index: u32,
    offset: usize,
    used: usize,
}

#[repr(u8)]
#[derive(Clone, Copy, PartialEq)]
pub enum Claim {
    None,
    Readers(u32),
    Writer,
}

impl Claim {
    pub fn try_read(&mut self) -> Result<()> {
        *self = match *self {
            Claim::None => Claim::Readers(1),
            Claim::Readers(n) => Claim::Readers(n + 1),
            Claim::Writer => return Err(Error::Busy),
        };
        Ok(())
    }

    pub fn try_write(&mut self) -> Result<()> {
        if *self != Claim::None {
            return Err(Error::Busy);
        }
        *self = Claim::Writer;
        Ok(())
    }

    pub fn end_read(&mut self) {
        if let Claim::Readers(n) = *self {
            *self = if n > 1 { Claim::Readers(n - 1) } else { Claim::None };
        }
    }

    pub fn end_reads(&mut self) {
        if let Claim::Readers(_) = *self {
            *self = Claim::None;
        }
    }
}

pub struct Super {
    sb: Superblock,
    disk: [u8; SUPERBLOCK_SIZE],
    dirty: bool,
}

impl Super {
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn discard(&mut self) {
        *self.sb.as_bytes_mut() = self.disk;
        self.dirty = false;
    }
}

impl Deref for Super {
    type Target = Superblock;

    fn deref(&self) -> &Superblock {
        &self.sb
    }
}

impl DerefMut for Super {
    fn deref_mut(&mut self) -> &mut Superblock {
        &mut self.sb
    }
}

pub struct BlockCache {
    bufs: [Buf; CACHE_SIZE],
    clock: u32,
}

impl BlockCache {
    pub fn invalidate(&mut self) {
        for b in &mut self.bufs {
            if b.refs == 0 {
                b.valid = false;
            }
        }
    }

    fn alloc(&mut self, block_size: u32) -> Result<()> {
        for b in &mut self.bufs {
            b.data = HeapBuf::new(block_size as usize).ok_or(Error::NoSpace)?;
        }
        Ok(())
    }

    fn free(&mut self) {
        for b in &mut self.bufs {
            b.data = HeapBuf::EMPTY;
        }
    }

    fn slot(&mut self, block: u32) -> usize {
        let mut victim: Option<usize> = None;
        for i in 0..CACHE_SIZE {
            let b = &self.bufs[i];
            if b.valid && b.block == block {
                self.clock = self.clock.wrapping_add(1);
                let b = &mut self.bufs[i];
                b.refs += 1;
                b.used = self.clock;
                return i;
            }
            if b.refs == 0 {
                let better = match victim {
                    None => true,
                    Some(v) => {
                        let v = &self.bufs[v];
                        !b.valid || (v.valid && b.used < v.used)
                    }
                };
                if better {
                    victim = Some(i);
                }
            }
        }
        let Some(victim) = victim else {
            unsafe { kprintf(c"ext2: block cache exhausted\n".as_ptr()) };
            halt();
        };
        self.clock = self.clock.wrapping_add(1);
        let b = &mut self.bufs[victim];
        b.valid = false;
        b.block = block;
        b.refs = 1;
        b.used = self.clock;
        victim
    }

    fn data(&mut self, b: usize) -> &mut [u8] {
        &mut self.bufs[b].data
    }

    fn release(&mut self, b: usize) {
        self.bufs[b].refs -= 1;
    }
}

pub struct Mount {
    pub dev: Disk,
    pub device: usize,
    pub path: Text<MOUNT_PATH_SIZE>,
    pub medium: Medium,
    pub generation: u32,
    pub nonce: u64,
    pub disk_head: u64,
    flushes: u32,
    pub claim: Claim,
    pub parked: bool,
    pub gone: bool,
    pub wrote: bool,
    pub writable: bool,
    last_alloc: u32,
    pub sb: Super,
    geo: Geometry,
    gdt: Groups,
    pub cache: BlockCache,
}

fn entry_size(name_len: usize) -> usize {
    (DIRENT_HEADER + name_len + 3) & !3
}

fn take_bit(bitmap: &mut [u8], count: u32, start: u32) -> Option<u32> {
    for n in 0..count {
        let bit = (start + n) % count;
        let (byte, mask) = ((bit / 8) as usize, 1u8 << (bit % 8));
        if bitmap[byte] & mask == 0 {
            bitmap[byte] |= mask;
            return Some(bit);
        }
    }
    None
}

fn clear_bit(bitmap: &mut [u8], bit: u32) {
    bitmap[(bit / 8) as usize] &= !(1u8 << (bit % 8));
}

fn set_bit(bitmap: &mut [u8], bit: u32) {
    bitmap[(bit / 8) as usize] |= 1u8 << (bit % 8);
}

fn now() -> u32 {
    rtc::seconds() as u32
}

pub fn read_head(dev: Disk) -> core::result::Result<(u64, bool), Fault> {
    let size = (dev.sectors() * SECTOR_SIZE as u64).min(HEAD_SIZE as u64) as usize;
    let mut raw = [0u8; 1024];
    let mut hash = FNV_OFFSET;
    let mut uniform = true;
    let mut first = 0;
    for at in (0..size).step_by(raw.len()) {
        let len = (size - at).min(raw.len());
        dev.read((at / SECTOR_SIZE) as u64, &mut raw[..len])?;
        if at == 0 {
            first = raw[0];
        }
        uniform &= raw[..len].iter().all(|&c| c == first);
        hash = fnv1a(hash, &raw[..len]);
    }
    Ok((hash, uniform && (first == 0x00 || first == FLOPPY_FILL)))
}

impl Mount {
    // ---------------------------------------------------------------------------------------- //
    // Blocks

    pub fn is_same_medium(&mut self) -> bool {
        loop {
            self.dev.ack_change();
            if self.dev.sectors() == 0 {
                return false;
            }
            let mut raw = [0u8; SUPERBLOCK_SIZE];
            let mut result = self.dev.read(SUPERBLOCK_OFFSET / SECTOR_SIZE as u64, &mut raw);
            if result.is_ok() && raw != self.sb.disk {
                return false;
            }
            if result.is_ok() && self.medium != Medium::Ext2 {
                match read_head(self.dev) {
                    Ok((head, _)) => return head == self.disk_head,
                    Err(e) => result = Err(e),
                }
            }
            if result == Err(Fault::Changed) {
                continue;
            }
            return result.is_ok();
        }
    }

    pub fn transfer(&mut self, sector: u64, mut data: Data) -> Result<()> {
        loop {
            if self.gone {
                return Err(Error::Changed);
            }
            let result = match &mut data {
                Data::Read(dst) => self.dev.read(sector, dst),
                Data::Write(src) => self.dev.write(sector, src),
            };
            match result {
                Ok(()) => {
                    self.wrote |= matches!(data, Data::Write(_));
                    return Ok(());
                }
                Err(Fault::Io) => return Err(Error::Io),
                Err(Fault::Changed) => {}
            }
            if !self.is_same_medium() {
                self.gone = true;
                return Err(Error::Changed);
            }
            self.cache.invalidate();
        }
    }

    fn read_sectors(&mut self, sector: u64, dst: &mut [u8]) -> Result<()> {
        self.transfer(sector, Data::Read(dst))
    }

    fn write_sectors(&mut self, sector: u64, src: &[u8]) -> Result<()> {
        self.transfer(sector, Data::Write(src))
    }

    fn block_sector(&self, block: u32) -> u64 {
        block as u64 * self.geo.sectors_per_block as u64
    }

    fn bread(&mut self, block: u32) -> Result<usize> {
        let b = self.cache.slot(block);
        if !self.cache.bufs[b].valid {
            let mut data = core::mem::take(&mut self.cache.bufs[b].data);
            let result = self.read_sectors(self.block_sector(block), &mut data);
            self.cache.bufs[b].data = data;
            if let Err(e) = result {
                self.cache.release(b);
                return Err(e);
            }
            self.cache.bufs[b].valid = true;
        }
        Ok(b)
    }

    fn bwrite(&mut self, b: usize) -> Result<()> {
        let data = core::mem::take(&mut self.cache.bufs[b].data);
        let result = self.write_sectors(self.block_sector(self.cache.bufs[b].block), &data);
        self.cache.bufs[b].data = data;
        result
    }

    fn bnew(&mut self, block: u32) -> usize {
        let b = self.cache.slot(block);
        self.cache.data(b).fill(0);
        self.cache.bufs[b].valid = true;
        b
    }

    fn zero_block(&mut self, block: u32) -> Result<()> {
        let b = self.bnew(block);
        let result = self.bwrite(b);
        self.cache.release(b);
        result
    }

    fn mark_group_dirty(&mut self, group: u32) {
        let per_block = self.geo.block_size as usize / size_of::<GroupDesc>();
        self.gdt.dirty[group as usize / per_block] = 1;
    }

    pub fn flush_metadata(&mut self) -> Result<()> {
        let mut result = Ok(());
        let block_size = self.geo.block_size as usize;
        for i in 0..self.geo.gdt_blocks as usize {
            if self.gdt.dirty[i] == 0 {
                continue;
            }
            let table = core::mem::take(&mut self.gdt.table);
            let sector = self.block_sector(self.sb.first_data_block + 1 + i as u32);
            let written = self.write_sectors(sector, &table[i * block_size..(i + 1) * block_size]);
            self.gdt.table = table;
            match written {
                Ok(()) => self.gdt.dirty[i] = 0,
                Err(e) => result = Err(e),
            }
        }
        let flushed = self.flush_super();
        result.and(flushed)
    }

    pub fn flush_super(&mut self) -> Result<()> {
        if !self.sb.dirty {
            return Ok(());
        }
        self.sb.wtime = now();
        self.flushes = self.flushes.wrapping_add(1);
        let mut stamp = Text::<64>::EMPTY;
        write!(stamp, "openos2 {:016x} {:08x}", self.nonce, self.flushes);
        let stamp = stamp.as_bytes();
        self.sb.last_mounted = [0; 64];
        self.sb.last_mounted[..stamp.len()].copy_from_slice(stamp);
        let raw = *self.sb.as_bytes();
        self.write_sectors(SUPERBLOCK_OFFSET / SECTOR_SIZE as u64, &raw)?;
        self.sb.dirty = false;
        self.sb.disk = raw;
        Ok(())
    }

    // ---------------------------------------------------------------------------------------- //
    // Allocation

    fn alloc_block(&mut self, zero: bool) -> Result<u32> {
        if self.sb.free_blocks_count == 0 {
            return Err(Error::NoSpace);
        }
        let (first_data_block, per_group) = (self.sb.first_data_block, self.sb.blocks_per_group);
        let goal = if self.last_alloc + 1 < self.sb.blocks_count { self.last_alloc + 1 } else { first_data_block };
        let goal_group = (goal - first_data_block) / per_group;
        for n in 0..self.geo.groups {
            let group = (goal_group + n) % self.geo.groups;
            let gd = *self.gdt.get(group);
            if gd.free_blocks_count == 0 {
                continue;
            }
            let first = first_data_block + group * per_group;
            let count = (self.sb.blocks_count - first).min(per_group);
            let b = self.bread(gd.block_bitmap)?;
            let Some(bit) = take_bit(self.cache.data(b), count, if n == 0 { goal - first } else { 0 }) else {
                self.cache.release(b);
                continue;
            };
            if let Err(e) = self.bwrite(b) {
                clear_bit(self.cache.data(b), bit);
                self.cache.release(b);
                return Err(e);
            }
            self.cache.release(b);
            self.gdt.get(group).free_blocks_count -= 1;
            self.sb.free_blocks_count -= 1;
            let block = first + bit;
            if zero {
                self.zero_block(block)?;
            }
            self.mark_group_dirty(group);
            self.sb.mark_dirty();
            self.last_alloc = block;
            return Ok(block);
        }
        Err(Error::NoSpace)
    }

    fn is_valid_block(&self, block: u32) -> bool {
        block >= self.sb.first_data_block && block < self.sb.blocks_count
    }

    fn is_valid_inode(&self, number: u32) -> bool {
        number >= 1 && number <= self.sb.inodes_count
    }

    fn free_block(&mut self, block: u32) {
        if !self.is_valid_block(block) {
            return;
        }
        let group = (block - self.sb.first_data_block) / self.sb.blocks_per_group;
        let bit = (block - self.sb.first_data_block) % self.sb.blocks_per_group;
        let bitmap = self.gdt.get(group).block_bitmap;
        let Ok(b) = self.bread(bitmap) else { return };
        clear_bit(self.cache.data(b), bit);
        if self.bwrite(b).is_err() {
            set_bit(self.cache.data(b), bit);
            self.cache.release(b);
            return;
        }
        self.cache.release(b);
        self.gdt.get(group).free_blocks_count += 1;
        self.sb.free_blocks_count += 1;
        self.mark_group_dirty(group);
        self.sb.mark_dirty();
    }

    fn inode_place(&mut self, number: u32) -> (u32, usize) {
        let group = (number - 1) / self.sb.inodes_per_group;
        let offset = ((number - 1) % self.sb.inodes_per_group) * self.geo.inode_size;
        let table = self.gdt.get(group).inode_table;
        (table + offset / self.geo.block_size, (offset % self.geo.block_size) as usize)
    }

    pub fn read_inode(&mut self, number: u32) -> Result<Node> {
        if !self.is_valid_inode(number) {
            return Err(Error::Io);
        }
        let (block, offset) = self.inode_place(number);
        let b = self.bread(block)?;
        let d = unsafe {
            self.cache.data(b)[offset..offset + size_of::<Inode>()].as_ptr().cast::<Inode>().read_unaligned()
        };
        self.cache.release(b);
        Ok(Node { number, d })
    }

    pub fn write_inode(&mut self, inode: &Node) -> Result<()> {
        if !self.is_valid_inode(inode.number) {
            return Err(Error::Io);
        }
        let (block, offset) = self.inode_place(inode.number);
        let b = self.bread(block)?;
        let slot = &mut self.cache.data(b)[offset..offset + size_of::<Inode>()];
        unsafe { slot.as_mut_ptr().cast::<Inode>().write_unaligned(inode.d) };
        let result = self.bwrite(b);
        self.cache.release(b);
        result
    }

    fn alloc_inode(&mut self, parent: u32, is_dir: bool) -> Result<Node> {
        if self.sb.free_inodes_count == 0 {
            return Err(Error::NoSpace);
        }
        let per_group = self.sb.inodes_per_group;
        let start = (parent - 1) / per_group;
        for n in 0..self.geo.groups {
            let group = (start + n) % self.geo.groups;
            let gd = *self.gdt.get(group);
            if gd.free_inodes_count == 0 {
                continue;
            }
            let b = self.bread(gd.inode_bitmap)?;
            let first_bit = if group == 0 { self.geo.first_ino - 1 } else { 0 };
            let bit = match take_bit(self.cache.data(b), per_group, first_bit) {
                Some(bit) if bit < first_bit => {
                    clear_bit(self.cache.data(b), bit);
                    self.cache.release(b);
                    continue;
                }
                None => {
                    self.cache.release(b);
                    continue;
                }
                Some(bit) => bit,
            };
            if let Err(e) = self.bwrite(b) {
                clear_bit(self.cache.data(b), bit);
                self.cache.release(b);
                return Err(e);
            }
            self.cache.release(b);
            let gd = self.gdt.get(group);
            gd.free_inodes_count -= 1;
            if is_dir {
                gd.used_dirs_count += 1;
            }
            self.sb.free_inodes_count -= 1;
            self.mark_group_dirty(group);
            self.sb.mark_dirty();

            let number = group * per_group + bit + 1;
            let (block, offset) = self.inode_place(number);
            let table = self.bread(block)?;
            let inode_size = self.geo.inode_size as usize;
            self.cache.data(table)[offset..offset + inode_size].fill(0);
            let _ = self.bwrite(table);
            self.cache.release(table);

            let now = now();
            let mut inode = Node { number, d: Inode::default() };
            inode.d.atime = now;
            inode.d.ctime = now;
            inode.d.mtime = now;
            return Ok(inode);
        }
        Err(Error::NoSpace)
    }

    pub fn free_inode(&mut self, inode: &mut Node) {
        let is_dir = inode.is_dir();
        inode.d.dtime = now();
        inode.d.links_count = 0;
        if self.write_inode(inode).is_err() {
            return;
        }
        let group = (inode.number - 1) / self.sb.inodes_per_group;
        let bit = (inode.number - 1) % self.sb.inodes_per_group;
        let bitmap = self.gdt.get(group).inode_bitmap;
        let Ok(b) = self.bread(bitmap) else { return };
        clear_bit(self.cache.data(b), bit);
        let _ = self.bwrite(b);
        self.cache.release(b);
        let gd = self.gdt.get(group);
        gd.free_inodes_count += 1;
        if is_dir {
            gd.used_dirs_count -= 1;
        }
        self.sb.free_inodes_count += 1;
        self.mark_group_dirty(group);
        self.sb.mark_dirty();
    }

    // ---------------------------------------------------------------------------------------- //
    // Block mapping

    fn i_blocks_per_block(&self) -> u32 {
        self.geo.block_size / I_BLOCK_SIZE
    }

    fn has_blocks(&self, inode: &Node) -> bool {
        let kind = inode.d.mode & S_IFMT;
        let attributes = if inode.d.file_acl != 0 { self.i_blocks_per_block() } else { 0 };
        kind == S_IFREG || kind == S_IFDIR || (kind == S_IFLNK && inode.d.blocks > attributes)
    }

    fn bmap(
        &mut self,
        inode: &mut Node,
        mut index: u32,
        allocate: bool,
        fresh: Option<&mut bool>,
    ) -> Result<Option<u32>> {
        let ppb = self.geo.pointers_per_block;
        let (slot, levels, first) = if index < 12 {
            (index as usize, 0, 0)
        } else if index - 12 < ppb {
            (12, 1, 12)
        } else if index - 12 - ppb < ppb * ppb {
            (13, 2, 12 + ppb)
        } else {
            (14, 3, 12 + ppb + ppb * ppb)
        };
        index -= first;
        let mut fresh = fresh;

        let mut block = inode.d.block[slot];
        if block != 0 && !self.is_valid_block(block) {
            return Err(Error::Io);
        }
        if block == 0 {
            if !allocate {
                return Ok(None);
            }
            block = self.alloc_block(levels > 0)?;
            inode.d.block[slot] = block;
            inode.d.blocks += self.i_blocks_per_block();
            if levels == 0
                && let Some(fresh) = fresh.as_deref_mut()
            {
                *fresh = true;
            }
        }

        for level in (1..=levels).rev() {
            let divisor = ppb.pow(level - 1);
            let b = self.bread(block)?;
            let entry = ((index / divisor) % ppb) as usize * 4;
            let mut next = le32(self.cache.data(b), entry).unwrap_or(0);
            if next == 0 && allocate {
                next = match self.alloc_block(level > 1) {
                    Ok(next) => next,
                    Err(e) => {
                        self.cache.release(b);
                        return Err(e);
                    }
                };
                self.cache.data(b)[entry..entry + 4].copy_from_slice(&next.to_le_bytes());
                inode.d.blocks += self.i_blocks_per_block();
                if level == 1
                    && let Some(fresh) = fresh.as_deref_mut()
                {
                    *fresh = true;
                }
                if let Err(e) = self.bwrite(b) {
                    self.cache.release(b);
                    return Err(e);
                }
            }
            self.cache.release(b);
            block = next;
            if block == 0 {
                return Ok(None);
            }
            if !self.is_valid_block(block) {
                return Err(Error::Io);
            }
        }
        Ok(Some(block))
    }

    fn free_tree(&mut self, inode: &mut Node, block: u32, level: u32) {
        if block == 0 || !self.is_valid_block(block) {
            return;
        }
        if level > 0
            && let Ok(b) = self.bread(block)
        {
            for i in 0..self.geo.pointers_per_block as usize {
                let entry = le32(self.cache.data(b), 4 * i).unwrap_or(0);
                self.free_tree(inode, entry, level - 1);
            }
            self.cache.release(b);
        }
        self.free_block(block);
        inode.d.blocks = inode.d.blocks.wrapping_sub(self.i_blocks_per_block());
    }

    pub fn truncate(&mut self, inode: &mut Node) -> Result<()> {
        let mut old = *inode;
        inode.d.block = [0; 15];
        inode.d.size = 0;
        inode.d.size_high = 0;
        inode.d.blocks = if inode.d.file_acl != 0 { self.i_blocks_per_block() } else { 0 };
        if let Err(e) = self.write_inode(inode) {
            *inode = old;
            return Err(e);
        }
        if self.has_blocks(&old) {
            let blocks = old.d.block;
            for &block in &blocks[..12] {
                self.free_tree(&mut old, block, 0);
            }
            self.free_tree(&mut old, blocks[12], 1);
            self.free_tree(&mut old, blocks[13], 2);
            self.free_tree(&mut old, blocks[14], 3);
        }
        Ok(())
    }

    // ---------------------------------------------------------------------------------------- //
    // Directories

    fn is_valid_entry(&self, e: Dirent, offset: usize) -> bool {
        let rec_len = e.rec_len as usize;
        rec_len >= DIRENT_HEADER
            && rec_len.is_multiple_of(4)
            && offset + rec_len <= self.geo.block_size as usize
            && DIRENT_HEADER + e.name_len as usize <= rec_len
    }

    fn block_count(&self, inode: &Node) -> u32 {
        (inode.d.size as u64).div_ceil(self.geo.block_size as u64) as u32
    }

    fn scan<R>(
        &mut self,
        dir: &mut Node,
        strict: bool,
        mut f: impl FnMut(&mut Self, usize, u32, usize, Option<usize>, Dirent) -> Option<Result<R>>,
    ) -> Result<Option<R>> {
        for i in 0..self.block_count(dir) {
            let Ok(Some(block)) = self.bmap(dir, i, false, None) else { continue };
            let b = self.bread(block)?;
            let mut prev = None;
            let mut offset = 0;
            let mut found = None;
            while offset + DIRENT_HEADER <= self.geo.block_size as usize {
                let e = Dirent::read(self.cache.data(b), offset);
                if !self.is_valid_entry(e, offset) {
                    if strict {
                        found = Some(Err(Error::Io));
                    }
                    break;
                }
                found = f(self, b, i, offset, prev, e);
                if found.is_some() {
                    break;
                }
                prev = Some(offset);
                offset += e.rec_len as usize;
            }
            self.cache.release(b);
            if let Some(found) = found {
                return found.map(Some);
            }
        }
        Ok(None)
    }

    pub fn dir_find(&mut self, dir: &mut Node, name: &[u8]) -> Result<Location> {
        let found = self.scan(dir, true, |m, b, block_index, offset, prev_offset, e| {
            (e.inode != 0 && e.name(m.cache.data(b), offset) == name).then_some(Ok(Location {
                block_index,
                offset,
                prev_offset,
                inode: e.inode,
                file_type: e.file_type,
            }))
        })?;
        found.ok_or(Error::NotFound)
    }

    pub fn dir_entries(
        &mut self,
        dir: &mut Node,
        mut f: impl FnMut(&mut Self, &[u8], u32, u8) -> Result<()>,
    ) -> Result<()> {
        self.scan(dir, false, |m, b, _, offset, _, e| {
            let mut name = [0u8; 255];
            let name = &mut name[..e.name_len as usize];
            name.copy_from_slice(e.name(m.cache.data(b), offset));
            if e.inode == 0 || name == b"." || name == b".." {
                return None;
            }
            f(m, name, e.inode, e.file_type).err().map(Err::<(), _>)
        })
        .map(|_| ())
    }

    pub fn is_dir_empty(&mut self, dir: &mut Node) -> bool {
        let found = self.scan(dir, false, |m, b, _, offset, _, e| {
            let name = e.name(m.cache.data(b), offset);
            (e.inode != 0 && name != b"." && name != b"..").then_some(Ok(()))
        });
        matches!(found, Ok(None))
    }

    fn fill_entry(&self, block: &mut [u8], at: usize, name: &[u8], inode: u32, file_type: u8) {
        let rec_len = Dirent::read(block, at).rec_len;
        let file_type = if self.geo.filetype { file_type } else { 0 };
        Dirent { inode, rec_len, name_len: name.len() as u8, file_type }.write(block, at);
        block[at + DIRENT_HEADER..at + DIRENT_HEADER + name.len()].copy_from_slice(name);
    }

    pub fn dir_reserve(&mut self, dir: &mut Node, len: usize) -> Result<Slot> {
        let need = entry_size(len);
        if dir.d.flags & INODE_INDEX_FL != 0 {
            dir.d.flags &= !INODE_INDEX_FL;
            self.write_inode(dir)?;
        }
        let found = self.scan(dir, true, |_, _, index, offset, _, e| {
            let used = if e.inode != 0 { entry_size(e.name_len as usize) } else { 0 };
            (e.rec_len as usize - used >= need).then_some(Ok(Slot { index, offset, used }))
        })?;
        if let Some(slot) = found {
            return Ok(slot);
        }

        let count = self.block_count(dir);
        let block = match self.bmap(dir, count, true, None) {
            Ok(Some(block)) => block,
            result => {
                let _ = self.write_inode(dir);
                return Err(result.err().unwrap_or(Error::NoSpace));
            }
        };
        let b = self.bnew(block);
        let block_size = self.geo.block_size as u16;
        self.cache.data(b)[4..6].copy_from_slice(&block_size.to_le_bytes());
        let result = self.bwrite(b);
        self.cache.release(b);
        result?;
        dir.d.size += self.geo.block_size;
        self.write_inode(dir)?;
        Ok(Slot { index: count, offset: 0, used: 0 })
    }

    fn update_block(&mut self, dir: &mut Node, index: u32, f: impl FnOnce(&Self, &mut [u8])) -> Result<()> {
        let Ok(Some(block)) = self.bmap(dir, index, false, None) else { return Err(Error::Io) };
        let b = self.bread(block)?;
        let mut data = core::mem::take(&mut self.cache.bufs[b].data);
        f(self, &mut data);
        self.cache.bufs[b].data = data;
        let result = self.bwrite(b);
        self.cache.release(b);
        result
    }

    pub fn dir_put(&mut self, dir: &mut Node, slot: Slot, name: &[u8], inode: u32, file_type: u8) -> Result<()> {
        self.update_block(dir, slot.index, |m, data| {
            let mut at = slot.offset;
            if slot.used != 0 {
                let mut e = Dirent::read(data, at);
                let next = at + slot.used;
                let mut n = Dirent::read(data, next);
                n.rec_len = e.rec_len - slot.used as u16;
                n.write(data, next);
                e.rec_len = slot.used as u16;
                e.write(data, at);
                at = next;
            }
            m.fill_entry(data, at, name, inode, file_type);
        })
    }

    pub fn dir_remove(&mut self, dir: &mut Node, loc: &Location) -> Result<()> {
        let result = self.update_block(dir, loc.block_index, |_, data| {
            let mut e = Dirent::read(data, loc.offset);
            match loc.prev_offset {
                Some(prev_offset) => {
                    let mut prev = Dirent::read(data, prev_offset);
                    prev.rec_len = prev.rec_len.wrapping_add(e.rec_len);
                    prev.write(data, prev_offset);
                }
                None => {
                    e.inode = 0;
                    e.write(data, loc.offset);
                }
            }
        });
        dir.d.flags &= !INODE_INDEX_FL;
        let now = now();
        dir.d.mtime = now;
        dir.d.ctime = now;
        result?;
        self.write_inode(dir)
    }

    pub fn dir_set_entry(&mut self, dir: &mut Node, loc: &Location, inode: u32, file_type: u8) -> Result<()> {
        let filetype = self.geo.filetype;
        self.update_block(dir, loc.block_index, |_, data| {
            let mut e = Dirent::read(data, loc.offset);
            e.inode = inode;
            e.file_type = if filetype { file_type } else { 0 };
            e.write(data, loc.offset);
        })
    }

    pub fn dir_set_parent(&mut self, dir: &mut Node, parent: u32) -> Result<()> {
        let loc = self.dir_find(dir, b"..")?;
        self.update_block(dir, loc.block_index, |_, data| {
            let mut e = Dirent::read(data, loc.offset);
            e.inode = parent;
            e.write(data, loc.offset);
        })
    }

    pub fn create(&mut self, path: &[u8], new: New) -> Result<Node> {
        let (mut parent, name) = self.lookup_parent(path)?;
        if !parent.is_dir() {
            return Err(Error::NotDir);
        }
        match self.dir_find(&mut parent, name) {
            Err(Error::NotFound) => {}
            Ok(_) => return Err(Error::Exists),
            Err(e) => return Err(e),
        }
        let slot = self.dir_reserve(&mut parent, name.len())?;

        let is_dir = matches!(new, New::Dir);
        let mut inode = self.alloc_inode(parent.number, is_dir)?;
        let file_type = match new {
            New::File => {
                inode.d.mode = S_IFREG | 0o644;
                inode.d.links_count = 1;
                FT_REG
            }
            New::Dir => {
                inode.d.mode = S_IFDIR | 0o755;
                inode.d.links_count = 2;
                let block = match self.bmap(&mut inode, 0, true, None) {
                    Ok(Some(block)) => block,
                    result => {
                        self.free_inode(&mut inode);
                        return Err(result.err().unwrap_or(Error::Io));
                    }
                };
                let b = self.bnew(block);
                let mut data = core::mem::take(&mut self.cache.bufs[b].data);
                Dirent { inode: 0, rec_len: 12, name_len: 0, file_type: 0 }.write(&mut data, 0);
                self.fill_entry(&mut data, 0, b".", inode.number, FT_DIR);
                Dirent { inode: 0, rec_len: (self.geo.block_size - 12) as u16, name_len: 0, file_type: 0 }
                    .write(&mut data, 12);
                self.fill_entry(&mut data, 12, b"..", parent.number, FT_DIR);
                self.cache.bufs[b].data = data;
                let result = self.bwrite(b);
                self.cache.release(b);
                result?;
                inode.d.size = self.geo.block_size;
                FT_DIR
            }
            New::Link(target) => {
                inode.d.mode = S_IFLNK | 0o777;
                inode.d.links_count = 1;
                if let Err(e) = self.write_link(&mut inode, target) {
                    self.free_unlinked(&mut inode);
                    return Err(e);
                }
                FT_SYMLINK
            }
        };
        self.write_inode(&inode)?;
        if is_dir {
            parent.d.links_count += 1;
            self.write_inode(&parent)?;
        }
        self.dir_put(&mut parent, slot, name, inode.number, file_type)?;
        Ok(inode)
    }

    pub fn link(&mut self, old: &[u8], new: &[u8]) -> Result<()> {
        let mut inode = self.lookup(old)?;
        if inode.is_dir() {
            return Err(Error::NotPermitted);
        }
        if inode.d.links_count >= LINK_MAX {
            return Err(Error::TooManyLinks);
        }
        let (mut old_parent, old_name) = self.lookup_parent(old)?;
        let file_type = self.dir_find(&mut old_parent, old_name)?.file_type;
        let (mut parent, name) = self.lookup_parent(new)?;
        if !parent.is_dir() {
            return Err(Error::NotDir);
        }
        match self.dir_find(&mut parent, name) {
            Err(Error::NotFound) if new.last() == Some(&b'/') => return Err(Error::NotFound),
            Err(Error::NotFound) => {}
            Ok(_) => return Err(Error::Exists),
            Err(e) => return Err(e),
        }
        let slot = self.dir_reserve(&mut parent, name.len())?;
        inode.d.links_count += 1;
        inode.d.ctime = now();
        self.write_inode(&inode)?;
        self.dir_put(&mut parent, slot, name, inode.number, file_type)
    }

    fn write_link(&mut self, inode: &mut Node, target: &[u8]) -> Result<()> {
        if target.len() >= 60 {
            return self.write_file(inode, &mut 0, target).1;
        }
        let mut inline = [0; 60];
        inline[..target.len()].copy_from_slice(target);
        inode.d.block = core::array::from_fn(|i| le32(&inline, 4 * i).unwrap_or(0));
        inode.d.size = target.len() as u32;
        Ok(())
    }

    pub fn unlink_inode(&mut self, inode: &mut Node, open: bool) {
        inode.d.links_count = inode.d.links_count.wrapping_sub(1);
        if inode.d.links_count == 0 && !open {
            self.free_unlinked(inode);
        } else {
            inode.d.ctime = now();
            let _ = self.write_inode(inode);
        }
    }

    pub fn free_unlinked(&mut self, inode: &mut Node) {
        inode.d.dtime = now();
        if self.truncate(inode).is_ok() {
            self.free_inode(inode);
        }
    }

    // ---------------------------------------------------------------------------------------- //
    // Paths

    fn walk<'a>(&mut self, path: &'a [u8], parent: bool) -> Result<(Node, &'a [u8])> {
        let mut inode = self.read_inode(ROOT_INODE)?;
        let mut at = 0;
        loop {
            let separator = at;
            while at < path.len() && path[at] == b'/' {
                at += 1;
            }
            if at == path.len() {
                if parent {
                    return Err(Error::Invalid);
                }
                if at != separator && at != 0 && !inode.is_dir() {
                    return Err(if inode.is_symlink() { vfs::link_at(separator) } else { Error::NotDir });
                }
                return Ok((inode, &[]));
            }
            let start = at;
            while at < path.len() && path[at] != b'/' {
                at += 1;
            }
            let name = &path[start..at];
            let last = path[at..].iter().all(|&c| c == b'/');
            if name.len() > 255 {
                return Err(Error::NameTooLong);
            }
            if !inode.is_dir() {
                return Err(if inode.is_symlink() { vfs::link_at(separator) } else { Error::NotDir });
            }
            if parent && last {
                if name == b"." || name == b".." {
                    return Err(Error::Invalid);
                }
                return Ok((inode, name));
            }
            let loc = self.dir_find(&mut inode, name)?;
            inode = self.read_inode(loc.inode)?;
        }
    }

    pub fn lookup(&mut self, path: &[u8]) -> Result<Node> {
        self.walk(path, false).map(|(inode, _)| inode)
    }

    pub fn lookup_parent<'a>(&mut self, path: &'a [u8]) -> Result<(Node, &'a [u8])> {
        self.walk(path, true)
    }

    // ---------------------------------------------------------------------------------------- //
    // Files

    pub fn read_file(&mut self, inode: &mut Node, pos: &mut u64, dst: &mut [u8]) -> Result<usize> {
        let mut done = 0;
        while done < dst.len() {
            let offset = (*pos % self.geo.block_size as u64) as usize;
            let chunk = (self.geo.block_size as usize - offset).min(dst.len() - done);
            let index = (*pos / self.geo.block_size as u64) as u32;
            match self.bmap(inode, index, false, None) {
                Err(e) => return if done > 0 { Ok(done) } else { Err(e) },
                Ok(None) => dst[done..done + chunk].fill(0),
                Ok(Some(block)) => {
                    let b = match self.bread(block) {
                        Ok(b) => b,
                        Err(e) => return if done > 0 { Ok(done) } else { Err(e) },
                    };
                    dst[done..done + chunk].copy_from_slice(&self.cache.data(b)[offset..offset + chunk]);
                    self.cache.release(b);
                }
            }
            done += chunk;
            *pos += chunk as u64;
        }
        Ok(done)
    }

    pub fn write_file(&mut self, inode: &mut Node, pos: &mut u64, src: &[u8]) -> (usize, Result<()>) {
        let mut done = 0;
        let mut result = Ok(());
        while done < src.len() {
            let offset = (*pos % self.geo.block_size as u64) as usize;
            let chunk = (self.geo.block_size as usize - offset).min(src.len() - done);
            let mut fresh = false;
            let block = self.bmap(inode, (*pos / self.geo.block_size as u64) as u32, true, Some(&mut fresh));
            let b = match block {
                Ok(None) => Err(Error::Io),
                Ok(Some(block)) if fresh || chunk == self.geo.block_size as usize => Ok(self.bnew(block)),
                Ok(Some(block)) => self.bread(block),
                Err(e) => Err(e),
            };
            let b = match b {
                Ok(b) => b,
                Err(e) => {
                    result = Err(e);
                    break;
                }
            };
            self.cache.data(b)[offset..offset + chunk].copy_from_slice(&src[done..done + chunk]);
            let written = self.bwrite(b);
            self.cache.release(b);
            if let Err(e) = written {
                result = Err(e);
                break;
            }
            done += chunk;
            *pos += chunk as u64;
            if *pos > inode.d.size as u64 {
                inode.d.size = *pos as u32;
            }
        }
        let now = now();
        inode.d.mtime = now;
        inode.d.ctime = now;
        let written = self.write_inode(inode);
        if done == 0 {
            return (0, result.and(written));
        }
        (done, Ok(()))
    }

    pub fn read_link(&mut self, inode: &mut Node, out: &mut [u8]) -> Result<usize> {
        let len = (inode.d.size as usize).min(out.len());
        if self.has_blocks(inode) {
            return self.read_file(inode, &mut 0, &mut out[..len]);
        }
        let inline: [u8; 60] = core::array::from_fn(|i| inode.d.block[i / 4].to_le_bytes()[i % 4]);
        let target = inline.get(..inode.d.size as usize).ok_or(Error::Io)?;
        out[..len].copy_from_slice(&target[..len]);
        Ok(len)
    }

    // ---------------------------------------------------------------------------------------- //
    // Mounting

    pub fn free_buffers(&mut self) {
        self.cache.free();
        self.gdt = Groups::default();
    }

    fn check_groups(&mut self) -> Result<()> {
        let table_blocks = (self.sb.inodes_per_group * self.geo.inode_size).div_ceil(self.geo.block_size);
        for group in 0..self.geo.groups {
            let gd = *self.gdt.get(group);
            if !self.is_valid_block(gd.block_bitmap)
                || !self.is_valid_block(gd.inode_bitmap)
                || !self.is_valid_block(gd.inode_table)
                || gd.inode_table as u64 + table_blocks as u64 > self.sb.blocks_count as u64
            {
                return Err(Error::Invalid);
            }
        }
        Ok(())
    }

    pub fn read_gdt(&mut self) -> Result<()> {
        let block_size = self.geo.block_size as usize;
        for i in 0..self.geo.gdt_blocks as usize {
            let mut table = core::mem::take(&mut self.gdt.table);
            let sector = self.block_sector(self.sb.first_data_block + 1 + i as u32);
            let result = self.read_sectors(sector, &mut table[i * block_size..(i + 1) * block_size]);
            self.gdt.table = table;
            result?;
        }
        self.check_groups()
    }

    pub fn is_dirty(&self) -> bool {
        self.sb.dirty || self.gdt.dirty.iter().any(|&b| b != 0)
    }

    pub fn read_mount(&mut self, dev: Disk) -> Result<()> {
        self.dev = dev;
        if dev.sectors() == 0 {
            return Err(Error::NotFound);
        }
        let mut raw = [0u8; SUPERBLOCK_SIZE];
        if self.read_sectors(SUPERBLOCK_OFFSET / SECTOR_SIZE as u64, &mut raw).is_err() {
            return Err(Error::Io);
        }
        *self.sb.as_bytes_mut() = raw;
        self.sb.disk = raw;
        let sb = *self.sb;
        self.medium = if sb.magic == MAGIC { Medium::Unsupported } else { Medium::Unknown };
        if sb.magic != MAGIC || sb.log_block_size > 2 || sb.rev_level > 1 {
            return Err(Error::Invalid);
        }
        let incompat = if sb.rev_level >= 1 { sb.feature_incompat } else { 0 };
        let ro_compat = if sb.rev_level >= 1 { sb.feature_ro_compat } else { 0 };
        if incompat & !INCOMPAT_FILETYPE != 0 {
            return Err(Error::Invalid);
        }
        self.writable = !dev.read_only() && ro_compat & !(RO_COMPAT_SPARSE_SUPER | RO_COMPAT_LARGE_FILE) == 0;
        self.geo = Geometry::new(&sb, dev.sectors())?;
        let (blocks, block_size) = (self.geo.gdt_blocks as usize, self.geo.block_size);
        self.gdt.table = HeapBuf::new(blocks * block_size as usize).ok_or(Error::NoSpace)?;
        self.gdt.dirty = HeapBuf::zeroed(blocks).ok_or(Error::NoSpace)?;
        self.read_gdt()?;
        self.medium = Medium::Ext2;
        self.cache.alloc(block_size)?;
        self.last_alloc = sb.first_data_block;
        Ok(())
    }

    pub fn label(&self) -> [u8; LABEL_LENGTH + 1] {
        let mut label = [0; LABEL_LENGTH + 1];
        if matches!(self.medium, Medium::Ext2 | Medium::Unsupported) {
            label[..LABEL_LENGTH].copy_from_slice(&self.sb.volume_name);
        }
        label
    }

    pub fn has_filetype(&self) -> bool {
        self.geo.filetype
    }

    pub fn total(&self) -> u64 {
        self.sb.blocks_count as u64 * self.geo.block_size as u64
    }

    pub fn free(&self) -> u64 {
        self.sb.free_blocks_count as u64 * self.geo.block_size as u64
    }
}

struct Geometry {
    block_size: u32,
    sectors_per_block: u32,
    pointers_per_block: u32,
    groups: u32,
    inode_size: u32,
    gdt_blocks: u32,
    first_ino: u32,
    filetype: bool,
}

impl Geometry {
    fn new(sb: &Superblock, sectors: u64) -> Result<Self> {
        let block_size = 1024 << sb.log_block_size;
        let sectors_per_block = block_size / SECTOR_SIZE as u32;
        let inode_size = if sb.rev_level >= 1 { sb.inode_size as u32 } else { 128 };
        let bits = 8 * block_size;
        if sb.blocks_per_group == 0
            || sb.blocks_per_group > bits
            || sb.inodes_per_group == 0
            || sb.inodes_per_group > bits
            || sb.first_data_block >= sb.blocks_count
            || inode_size < 128
            || inode_size > block_size
            || !inode_size.is_power_of_two()
            || sb.blocks_count as u64 * sectors_per_block as u64 > sectors
        {
            return Err(Error::Invalid);
        }
        let groups = (sb.blocks_count - sb.first_data_block).div_ceil(sb.blocks_per_group);
        if sb.inodes_count as u64 > groups as u64 * sb.inodes_per_group as u64 {
            return Err(Error::Invalid);
        }
        Ok(Self {
            block_size,
            sectors_per_block,
            pointers_per_block: block_size / 4,
            groups,
            inode_size,
            gdt_blocks: groups.div_ceil(block_size / size_of::<GroupDesc>() as u32),
            first_ino: if sb.rev_level >= 1 { sb.first_ino } else { 11 },
            filetype: sb.rev_level >= 1 && sb.feature_incompat & INCOMPAT_FILETYPE != 0,
        })
    }
}

#[derive(Default)]
struct Groups {
    table: HeapBuf,
    dirty: HeapBuf, // per descriptor table block
}

impl Groups {
    fn get(&mut self, group: u32) -> &mut GroupDesc {
        unsafe { &mut self.table.align_to_mut::<GroupDesc>().1[group as usize] }
    }
}

#[derive(Default)]
struct Buf {
    block: u32,
    valid: bool,
    refs: u32,
    used: u32,
    data: HeapBuf,
}
