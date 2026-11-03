// SPDX-License-Identifier: MIT

pub const MAGIC: u16 = 0xEF53;
pub const SUPERBLOCK_OFFSET: u64 = 1024;
pub const SUPERBLOCK_SIZE: usize = 1024;
pub const I_BLOCK_SIZE: u32 = 512; // the unit of i_blocks
pub const ROOT_INODE: u32 = 2;
pub const S_IFMT: u16 = 0xF000;
pub const S_IFDIR: u16 = 0x4000;
pub const S_IFREG: u16 = 0x8000;
pub const S_IFLNK: u16 = 0xA000;
pub const FT_REG: u8 = 1;
pub const FT_DIR: u8 = 2;
pub const FT_SYMLINK: u8 = 7;
pub const LINK_MAX: u16 = 32000; // Linux's EXT2_LINK_MAX
pub const INCOMPAT_FILETYPE: u32 = 0x2;
pub const RO_COMPAT_SPARSE_SUPER: u32 = 0x1;
pub const RO_COMPAT_LARGE_FILE: u32 = 0x2;
pub const VOLUME_NAME_OFFSET: usize = 120;
pub const HEAD_SIZE: usize = 16 * 1024; // FS_MEDIUM_BLANK if uniform, written last by mkfs
pub const STATE_CLEAN: u16 = 1;
pub const STATE_ERROR: u16 = 2;
pub const DIRENT_HEADER: usize = 8;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Superblock {
    pub inodes_count: u32,
    pub blocks_count: u32,
    pub reserved_blocks_count: u32,
    pub free_blocks_count: u32,
    pub free_inodes_count: u32,
    pub first_data_block: u32,
    pub log_block_size: u32,
    pub log_frag_size: u32,
    pub blocks_per_group: u32,
    pub frags_per_group: u32,
    pub inodes_per_group: u32,
    pub mtime: u32,
    pub wtime: u32,
    pub mnt_count: u16,
    pub max_mnt_count: u16,
    pub magic: u16,
    pub state: u16,
    pub errors: u16,
    pub minor_rev_level: u16,
    pub lastcheck: u32,
    pub checkinterval: u32,
    pub creator_os: u32,
    pub rev_level: u32,
    pub def_resuid: u16,
    pub def_resgid: u16,
    pub first_ino: u32,
    pub inode_size: u16,
    pub block_group_nr: u16,
    pub feature_compat: u32,
    pub feature_incompat: u32,
    pub feature_ro_compat: u32,
    pub uuid: [u8; 16],
    pub volume_name: [u8; 16],
    pub last_mounted: [u8; 64],
    pub rest: [u8; 824],
}

impl Superblock {
    pub fn as_bytes(&self) -> &[u8; SUPERBLOCK_SIZE] {
        unsafe { &*(self as *const Self).cast() }
    }

    pub fn as_bytes_mut(&mut self) -> &mut [u8; SUPERBLOCK_SIZE] {
        unsafe { &mut *(self as *mut Self).cast() }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GroupDesc {
    pub block_bitmap: u32,
    pub inode_bitmap: u32,
    pub inode_table: u32,
    pub free_blocks_count: u16,
    pub free_inodes_count: u16,
    pub used_dirs_count: u16,
    pub pad: u16,
    pub reserved: [u32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Inode {
    pub mode: u16,
    pub uid: u16,
    pub size: u32,
    pub atime: u32,
    pub ctime: u32,
    pub mtime: u32,
    pub dtime: u32,
    pub gid: u16,
    pub links_count: u16,
    pub blocks: u32,
    pub flags: u32,
    pub osd1: u32,
    pub block: [u32; 15],
    pub generation: u32,
    pub file_acl: u32,
    pub size_high: u32,
    pub faddr: u32,
    pub osd2: [u8; 12],
}

const _: () = assert!(size_of::<Superblock>() == SUPERBLOCK_SIZE);
const _: () = assert!(size_of::<GroupDesc>() == 32);
const _: () = assert!(size_of::<Inode>() == 128);

#[derive(Clone, Copy)]
pub struct Dirent {
    pub inode: u32,
    pub rec_len: u16,
    pub name_len: u8,
    pub file_type: u8,
}

impl Dirent {
    pub fn read(block: &[u8], at: usize) -> Self {
        let b = &block[at..at + DIRENT_HEADER];
        Self {
            inode: u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            rec_len: u16::from_le_bytes([b[4], b[5]]),
            name_len: b[6],
            file_type: b[7],
        }
    }

    pub fn write(self, block: &mut [u8], at: usize) {
        block[at..at + 4].copy_from_slice(&self.inode.to_le_bytes());
        block[at + 4..at + 6].copy_from_slice(&self.rec_len.to_le_bytes());
        block[at + 6] = self.name_len;
        block[at + 7] = self.file_type;
    }

    pub fn name(self, block: &[u8], at: usize) -> &[u8] {
        &block[at + DIRENT_HEADER..at + DIRENT_HEADER + self.name_len as usize]
    }
}
