// SPDX-License-Identifier: MIT

use super::disk::{
    DIRENT_HEADER, FT_DIR, HEAD_SIZE, I_BLOCK_SIZE, INCOMPAT_FILETYPE, MAGIC, RO_COMPAT_SPARSE_SUPER, ROOT_INODE,
    S_IFDIR, STATE_CLEAN, SUPERBLOCK_OFFSET, VOLUME_NAME_OFFSET,
};
use crate::fs::{Error, LABEL_LENGTH, Result};
use crate::mem::HeapBuf;

const SECTOR_SIZE: usize = 512;
const BLOCK_SIZE: usize = 1024;
const BLOCKS_PER_GROUP: u32 = 8 * BLOCK_SIZE as u32;
const INODE_SIZE: usize = 256;
const INODES_PER_BLOCK: u32 = (BLOCK_SIZE / INODE_SIZE) as u32;
const GROUP_DESC_SIZE: usize = 32;
const EXTRA_ISIZE: u32 = 32;
const FIRST_INO: u32 = 11;
const LOST_FOUND_INO: u32 = 11;
const LOST_FOUND_BLOCKS: u32 = 12;
const MIN_BLOCKS: u32 = 60;
const MIB: u64 = 1024 * 1024;
const HEAD_BLOCKS: u32 = (HEAD_SIZE / BLOCK_SIZE) as u32;
const FIRST_DATA_BLOCK: u32 = (SUPERBLOCK_OFFSET / BLOCK_SIZE as u64) as u32;

const S_INODES_COUNT: usize = 0x00;
const S_BLOCKS_COUNT: usize = 0x04;
const S_R_BLOCKS_COUNT: usize = 0x08;
const S_FREE_BLOCKS_COUNT: usize = 0x0C;
const S_FREE_INODES_COUNT: usize = 0x10;
const S_FIRST_DATA_BLOCK: usize = 0x14;
const S_BLOCKS_PER_GROUP: usize = 0x20;
const S_FRAGS_PER_GROUP: usize = 0x24;
const S_INODES_PER_GROUP: usize = 0x28;
const S_WTIME: usize = 0x30;
const S_MAX_MNT_COUNT: usize = 0x36;
const S_MAGIC: usize = 0x38;
const S_STATE: usize = 0x3A;
const S_ERRORS: usize = 0x3C;
const S_LASTCHECK: usize = 0x40;
const S_REV_LEVEL: usize = 0x4C;
const S_FIRST_INO: usize = 0x54;
const S_INODE_SIZE: usize = 0x58;
const S_BLOCK_GROUP_NR: usize = 0x5A;
const S_FEATURE_INCOMPAT: usize = 0x60;
const S_FEATURE_RO_COMPAT: usize = 0x64;
const S_UUID: usize = 0x68;
const S_HASH_SEED: usize = 0xEC;
const S_DEF_HASH_VERSION: usize = 0xFC;
const S_DEFAULT_MOUNT_OPTS: usize = 0x100;
const S_MKFS_TIME: usize = 0x108;
const S_MIN_EXTRA_ISIZE: usize = 0x15C;
const S_WANT_EXTRA_ISIZE: usize = 0x15E;
const S_FLAGS: usize = 0x160;

const G_BLOCK_BITMAP: usize = 0x00;
const G_INODE_BITMAP: usize = 0x04;
const G_INODE_TABLE: usize = 0x08;
const G_FREE_BLOCKS_COUNT: usize = 0x0C;
const G_FREE_INODES_COUNT: usize = 0x0E;
const G_USED_DIRS_COUNT: usize = 0x10;
const G_FLAGS: usize = 0x12;
const BG_INODE_ZEROED: u32 = 0x4;

const I_MODE: usize = 0x00;
const I_SIZE: usize = 0x04;
const I_ATIME: usize = 0x08;
const I_CTIME: usize = 0x0C;
const I_MTIME: usize = 0x10;
const I_LINKS_COUNT: usize = 0x1A;
const I_BLOCKS: usize = 0x1C;
const I_BLOCK: usize = 0x28;
const I_EXTRA_ISIZE: usize = 0x80;
const I_CRTIME: usize = 0x90;

const ERRORS_CONTINUE: u32 = 1;
const HASH_HALF_MD4: u8 = 1;
const DEFAULT_MOUNT_XATTR_USER: u32 = 0x4;
const DEFAULT_MOUNT_ACL: u32 = 0x8;
const FLAGS_SIGNED_HASH: u32 = 0x1;

pub trait Medium {
    fn sectors(&self) -> u64;
    fn read_only(&self) -> bool;
    fn write(&mut self, sector: u64, data: &[u8]) -> Result<()>;
}

struct Layout {
    blocks: u32,
    groups: u32,
    inodes_per_group: u32,
    table_blocks: u32,
    gdt_blocks: u32,
}

struct Pass<'a, M: Medium> {
    dev: &'a mut M,
    head: Option<&'a mut [u8]>,
}

fn put16(p: &mut [u8], offset: usize, value: u32) {
    p[offset..offset + 2].copy_from_slice(&(value as u16).to_le_bytes());
}

fn put32(p: &mut [u8], offset: usize, value: u32) {
    p[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn group_start(group: u32) -> u32 {
    FIRST_DATA_BLOCK + group * BLOCKS_PER_GROUP
}

fn is_power_of(mut n: u32, base: u32) -> bool {
    while n.is_multiple_of(base) {
        n /= base;
    }
    n == 1
}

fn has_super(group: u32) -> bool {
    group <= 1 || is_power_of(group, 3) || is_power_of(group, 5) || is_power_of(group, 7)
}

impl Layout {
    fn group_blocks(&self, group: u32) -> u32 {
        (self.blocks - group_start(group)).min(BLOCKS_PER_GROUP)
    }

    fn overhead(&self, group: u32) -> u32 {
        (if has_super(group) { 1 + self.gdt_blocks } else { 0 }) + 2 + self.table_blocks
    }

    fn plan(sectors: u64) -> Result<Self> {
        let bytes = sectors * SECTOR_SIZE as u64;
        let mut l = Layout { blocks: 0, groups: 0, inodes_per_group: 0, table_blocks: 0, gdt_blocks: 0 };
        l.blocks = (bytes / BLOCK_SIZE as u64).min(u32::MAX as u64) as u32 & !3;
        let ratio = if bytes < 3 * MIB {
            8192
        } else if bytes < 512 * MIB {
            4096
        } else {
            16384
        };
        loop {
            if l.blocks < MIN_BLOCKS {
                return Err(Error::NoSpace);
            }
            l.groups = (l.blocks - FIRST_DATA_BLOCK).div_ceil(BLOCKS_PER_GROUP);
            let inodes = l.blocks as u64 * BLOCK_SIZE as u64 / ratio;
            let per_group =
                (inodes.div_ceil(l.groups as u64).div_ceil(INODES_PER_BLOCK as u64) * INODES_PER_BLOCK as u64) & !7;
            l.inodes_per_group = per_group.clamp(16, 8 * BLOCK_SIZE as u64) as u32;
            l.table_blocks = l.inodes_per_group / INODES_PER_BLOCK;
            l.gdt_blocks = (l.groups as usize * GROUP_DESC_SIZE).div_ceil(BLOCK_SIZE) as u32;
            let last = l.groups - 1;
            if l.groups > 1 && l.group_blocks(last) < l.overhead(last) + 50 {
                l.blocks -= l.group_blocks(last);
                continue;
            }
            if l.overhead(0) + 1 + LOST_FOUND_BLOCKS >= l.group_blocks(0) {
                return Err(Error::NoSpace);
            }
            return Ok(l);
        }
    }

    fn root_block(&self) -> u32 {
        group_start(0) + self.overhead(0)
    }
}

impl<M: Medium> Pass<'_, M> {
    fn write_block(&mut self, block: u32, data: &[u8]) -> Result<()> {
        match &mut self.head {
            Some(head) if block < HEAD_BLOCKS => {
                let at = block as usize * BLOCK_SIZE;
                head[at..at + BLOCK_SIZE].copy_from_slice(&data[..BLOCK_SIZE]);
                Ok(())
            }
            None if block >= HEAD_BLOCKS => {
                self.dev.write(block as u64 * (BLOCK_SIZE / SECTOR_SIZE) as u64, &data[..BLOCK_SIZE])
            }
            _ => Ok(()),
        }
    }

    fn write_groups(&mut self, l: &Layout, gdt: &mut [u8], buf: &mut [u8], now: u32) -> Result<()> {
        let root_block = l.root_block();
        for group in 0..l.groups {
            let block_bitmap = group_start(group) + if has_super(group) { 1 + l.gdt_blocks } else { 0 };
            let inode_bitmap = block_bitmap + 1;
            let inode_table = inode_bitmap + 1;
            let used_blocks = l.overhead(group) + if group == 0 { 1 + LOST_FOUND_BLOCKS } else { 0 };
            let used_inodes = if group == 0 { FIRST_INO } else { 0 };
            let gd = &mut gdt[group as usize * GROUP_DESC_SIZE..];
            put32(gd, G_BLOCK_BITMAP, block_bitmap);
            put32(gd, G_INODE_BITMAP, inode_bitmap);
            put32(gd, G_INODE_TABLE, inode_table);
            put16(gd, G_FREE_BLOCKS_COUNT, l.group_blocks(group) - used_blocks);
            put16(gd, G_FREE_INODES_COUNT, l.inodes_per_group - used_inodes);
            put16(gd, G_USED_DIRS_COUNT, if group == 0 { 2 } else { 0 });
            put16(gd, G_FLAGS, BG_INODE_ZEROED);

            fill_bitmap(buf, used_blocks, l.group_blocks(group));
            self.write_block(block_bitmap, buf)?;
            fill_bitmap(buf, used_inodes, l.inodes_per_group);
            self.write_block(inode_bitmap, buf)?;
            for i in 0..l.table_blocks {
                buf.fill(0);
                if group == 0 && i == 0 {
                    put32(buf, I_ATIME, now);
                    put32(buf, I_CTIME, now);
                    put32(buf, I_MTIME, now);
                    let root = (ROOT_INODE as usize - 1) * INODE_SIZE;
                    put_dir_inode(&mut buf[root..], (S_IFDIR | 0o755) as u32, 3, root_block, 1, now);
                }
                if group == 0 && i == (LOST_FOUND_INO - 1) / INODES_PER_BLOCK {
                    let lost_found = ((LOST_FOUND_INO - 1) % INODES_PER_BLOCK) as usize * INODE_SIZE;
                    put_dir_inode(
                        &mut buf[lost_found..],
                        (S_IFDIR | 0o700) as u32,
                        2,
                        root_block + 1,
                        LOST_FOUND_BLOCKS,
                        now,
                    );
                }
                self.write_block(inode_table + i, buf)?;
            }
        }
        Ok(())
    }

    fn write_directories(&mut self, l: &Layout, buf: &mut [u8]) -> Result<()> {
        let root_block = l.root_block();
        buf.fill(0);
        let offset = put_entry(buf, 0, ROOT_INODE, b".", false);
        let offset = put_entry(buf, offset, ROOT_INODE, b"..", false);
        put_entry(buf, offset, LOST_FOUND_INO, b"lost+found", true);
        self.write_block(root_block, buf)?;
        for i in 0..LOST_FOUND_BLOCKS {
            buf.fill(0);
            if i == 0 {
                let offset = put_entry(buf, 0, LOST_FOUND_INO, b".", false);
                put_entry(buf, offset, ROOT_INODE, b"..", true);
            } else {
                put16(buf, 4, BLOCK_SIZE as u32);
            }
            self.write_block(root_block + 1 + i, buf)?;
        }
        Ok(())
    }

    fn write_metadata(&mut self, l: &Layout, gdt: &[u8], sb: &mut [u8]) -> Result<()> {
        for group in (0..l.groups).rev() {
            if !has_super(group) {
                continue;
            }
            for i in 0..l.gdt_blocks {
                self.write_block(group_start(group) + 1 + i, &gdt[i as usize * BLOCK_SIZE..])?;
            }
            put16(sb, S_BLOCK_GROUP_NR, group);
            put16(sb, S_STATE, if group == 0 { STATE_CLEAN as u32 } else { 0 });
            self.write_block(group_start(group), sb)?;
        }
        Ok(())
    }
}

fn fill_bitmap(bitmap: &mut [u8], used: u32, valid: u32) {
    bitmap[..BLOCK_SIZE].fill(0);
    for bit in 0..8 * BLOCK_SIZE as u32 {
        if bit < used || bit >= valid {
            bitmap[(bit / 8) as usize] |= 1 << (bit % 8);
        }
    }
}

fn put_dir_inode(inode: &mut [u8], mode: u32, links: u32, first: u32, count: u32, now: u32) {
    put16(inode, I_MODE, mode);
    put32(inode, I_SIZE, count * BLOCK_SIZE as u32);
    put32(inode, I_ATIME, now);
    put32(inode, I_CTIME, now);
    put32(inode, I_MTIME, now);
    put16(inode, I_LINKS_COUNT, links);
    put32(inode, I_BLOCKS, count * (BLOCK_SIZE as u32 / I_BLOCK_SIZE));
    for i in 0..count {
        put32(inode, I_BLOCK + 4 * i as usize, first + i);
    }
    put16(inode, I_EXTRA_ISIZE, EXTRA_ISIZE);
    put32(inode, I_CRTIME, now);
}

fn put_entry(block: &mut [u8], offset: usize, inode: u32, name: &[u8], last: bool) -> usize {
    let rec_len = if last { BLOCK_SIZE - offset } else { (DIRENT_HEADER + name.len()).div_ceil(4) * 4 };
    put32(block, offset, inode);
    put16(block, offset + 4, rec_len as u32);
    block[offset + 6] = name.len() as u8;
    block[offset + 7] = FT_DIR;
    block[offset + DIRENT_HEADER..offset + DIRENT_HEADER + name.len()].copy_from_slice(name);
    offset + rec_len
}

fn put_super(sb: &mut [u8], l: &Layout, label: &[u8], now: u32, mut random: impl FnMut(u64) -> u64) {
    let mut free_blocks = 0;
    for group in 0..l.groups {
        free_blocks += l.group_blocks(group) - l.overhead(group);
    }
    free_blocks -= 1 + LOST_FOUND_BLOCKS;
    sb.fill(0);
    put32(sb, S_INODES_COUNT, l.groups * l.inodes_per_group);
    put32(sb, S_BLOCKS_COUNT, l.blocks);
    put32(sb, S_R_BLOCKS_COUNT, l.blocks / 20);
    put32(sb, S_FREE_BLOCKS_COUNT, free_blocks);
    put32(sb, S_FREE_INODES_COUNT, l.groups * l.inodes_per_group - FIRST_INO);
    put32(sb, S_FIRST_DATA_BLOCK, FIRST_DATA_BLOCK);
    put32(sb, S_BLOCKS_PER_GROUP, BLOCKS_PER_GROUP);
    put32(sb, S_FRAGS_PER_GROUP, BLOCKS_PER_GROUP);
    put32(sb, S_INODES_PER_GROUP, l.inodes_per_group);
    put32(sb, S_WTIME, now);
    put16(sb, S_MAX_MNT_COUNT, 0xFFFF);
    put16(sb, S_MAGIC, MAGIC as u32);
    put16(sb, S_STATE, STATE_CLEAN as u32);
    put16(sb, S_ERRORS, ERRORS_CONTINUE);
    put32(sb, S_LASTCHECK, now);
    put32(sb, S_REV_LEVEL, 1);
    put32(sb, S_FIRST_INO, FIRST_INO);
    put16(sb, S_INODE_SIZE, INODE_SIZE as u32);
    put32(sb, S_FEATURE_INCOMPAT, INCOMPAT_FILETYPE);
    put32(sb, S_FEATURE_RO_COMPAT, RO_COMPAT_SPARSE_SUPER);
    for i in 0..4 {
        put32(sb, S_UUID + 4 * i, random(i as u64) as u32);
        put32(sb, S_HASH_SEED + 4 * i, random(i as u64 + 4) as u32);
    }
    sb[S_UUID + 6] = (sb[S_UUID + 6] & 0x0F) | 0x40;
    sb[S_UUID + 8] = (sb[S_UUID + 8] & 0x3F) | 0x80;
    let label = &label[..label.len().min(LABEL_LENGTH)];
    sb[VOLUME_NAME_OFFSET..VOLUME_NAME_OFFSET + label.len()].copy_from_slice(label);
    sb[S_DEF_HASH_VERSION] = HASH_HALF_MD4;
    put32(sb, S_DEFAULT_MOUNT_OPTS, DEFAULT_MOUNT_XATTR_USER | DEFAULT_MOUNT_ACL);
    put32(sb, S_MKFS_TIME, now);
    put16(sb, S_MIN_EXTRA_ISIZE, EXTRA_ISIZE);
    put16(sb, S_WANT_EXTRA_ISIZE, EXTRA_ISIZE);
    put32(sb, S_FLAGS, FLAGS_SIGNED_HASH);
}

pub fn format(dev: &mut impl Medium, label: &[u8], now: u32, random: impl FnMut(u64) -> u64) -> Result<()> {
    if dev.read_only() {
        return Err(Error::ReadOnly);
    }
    let l = Layout::plan(dev.sectors())?;
    let buffers = (
        HeapBuf::zeroed(BLOCK_SIZE),
        HeapBuf::new(BLOCK_SIZE),
        HeapBuf::zeroed(l.gdt_blocks as usize * BLOCK_SIZE),
        HeapBuf::zeroed(HEAD_SIZE),
    );
    let (Some(mut buf), Some(mut sb), Some(mut gdt), Some(mut head)) = buffers else {
        return Err(Error::NoSpace);
    };
    put_super(&mut sb, &l, label, now, random);
    let head_size = l.blocks.min(HEAD_BLOCKS) as usize * BLOCK_SIZE;
    dev.write(0, &head[..head_size])?;
    for last in [false, true] {
        let mut pass = Pass { dev: &mut *dev, head: if last { Some(&mut head[..]) } else { None } };
        gdt.fill(0);
        pass.write_groups(&l, &mut gdt, &mut buf, now)?;
        pass.write_directories(&l, &mut buf)?;
        pass.write_metadata(&l, &gdt, &mut sb)?;
    }
    dev.write(0, &head[..head_size])
}
