/* SPDX-License-Identifier: MIT */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "drivers/rtc.h"
#include "drivers/virtio/virtio.h"
#include "ext2.h"
#include "internal.h"
#include "p9.h"

#define SUPERBLOCK_OFFSET 1024
#define MAX_BLOCK_SIZE 4096
#define SECTOR_SIZE 512

#define COMPAT_DIR_INDEX 0x20
#define RO_COMPAT_LARGE_FILE 0x2

#define S_IFMT 0xF000
#define S_IFDIR 0x4000
#define S_IFREG 0x8000
#define S_IFLNK 0xA000
#define FAST_SYMLINK_MAX 60
#define FT_REG 1
#define INODE_INDEX_FL 0x1000
#define STATE_CLEAN 1
#define LAST_MOUNTED_OFFSET 136
#define LAST_MOUNTED_SIZE 64
#define STATE_ERROR 2

#define CACHE_SIZE 12
#define MAX_FILES 16
#define MAX_RAW 2
#define HEAD_SIZE (16 * 1024)
#define FLOPPY_FILL 0xE5
#define RAW_CHUNK 0x10000
#define MAX_PATH_DEPTH 64

struct superblock {
    u32 inodes_count;
    u32 blocks_count;
    u32 reserved_blocks_count;
    u32 free_blocks_count;
    u32 free_inodes_count;
    u32 first_data_block;
    u32 log_block_size;
    u32 log_frag_size;
    u32 blocks_per_group;
    u32 frags_per_group;
    u32 inodes_per_group;
    u32 mtime;
    u32 wtime;
    u16 mnt_count;
    u16 max_mnt_count;
    u16 magic;
    u16 state;
    u16 errors;
    u16 minor_rev_level;
    u32 lastcheck;
    u32 checkinterval;
    u32 creator_os;
    u32 rev_level;
    u16 def_resuid;
    u16 def_resgid;
    u32 first_ino;
    u16 inode_size;
    u16 block_group_nr;
    u32 feature_compat;
    u32 feature_incompat;
    u32 feature_ro_compat;
};

struct group_desc {
    u32 block_bitmap;
    u32 inode_bitmap;
    u32 inode_table;
    u16 free_blocks_count;
    u16 free_inodes_count;
    u16 used_dirs_count;
    u16 pad;
    u32 reserved[3];
};

struct disk_inode {
    u16 mode;
    u16 uid;
    u32 size;
    u32 atime;
    u32 ctime;
    u32 mtime;
    u32 dtime;
    u16 gid;
    u16 links_count;
    u32 blocks;
    u32 flags;
    u32 osd1;
    u32 block[15];
    u32 generation;
    u32 file_acl;
    u32 size_high;
    u32 faddr;
    u8 osd2[12];
};

struct inode {
    u32 number;
    struct disk_inode d;
};

struct dirent {
    u32 inode;
    u16 rec_len;
    u8 name_len;
    u8 file_type;
    char name[];
};

struct buf {
    u32 block;
    int valid;
    int refs;
    u32 used;
    u8 *data;
};

struct mount;

struct file {
    int open;
    int flags;
    u32 inode;
    u64 pos;
    struct mount *mount; /* NULL once its medium was changed */
};

struct mount {
    struct vblk *dev;
    int device; /* index of dev, see vblk_get */
    char path[6 + FS_LABEL_LENGTH]; /* "/", "/mnt/<label>" or "/mnt/<device>" */
    int medium; /* FS_MEDIUM_*, for any but ext2 only dev, device, path, writable, disk_* are set */
    u32 serial;
    int raw_readers; /* see fs_open_raw: nothing may be written meanwhile */
    int detached; /* open for a raw write, see fs_open_raw */
    int writable;
    int filetype;
    u32 block_size;
    u32 sectors_per_block;
    u32 pointers_per_block;
    u32 groups;
    u32 inode_size;
    u32 gdt_blocks;
    u32 last_alloc;
    u32 first_ino;
    union {
        struct superblock sb;
        u8 raw[1024];
    } super;
    int super_dirty;
    u8 disk_super[1024]; /* the superblock as last read from or written to the medium */
    u64 disk_head; /* a hash of the first HEAD_SIZE bytes, for media that are not ext2, see is_same_medium */
    int gone; /* the medium was replaced, nothing more is written */
    u64 nonce; /* random per read-write mount, see stamp */
    u32 flushes; /* superblock writes since the mount */
    int wrote; /* something was written during the current call */
    struct group_desc *gdt;
    u8 *gdt_dirty; /* per descriptor table block */
    struct buf cache[CACHE_SIZE];
    u32 cache_clock;
};

static struct mount *fs;
static struct mount *mounts[MAX_VIRTIO];
static struct mount *root;
static int boot_device = -1;
static struct file files[MAX_FILES];
static struct raw {
    int open;
    int write;
    u64 pos;
    struct mount *mount; /* NULL once its medium was changed */
} raws[MAX_RAW];
static u32 serials;

#define SB (&fs->super.sb)

/* ---------------------------------------------------------------------------------------- */
/* Blocks */

static void drop_cache(void) {
    for (int i = 0; i < CACHE_SIZE; i++) {
        if (fs->cache[i].refs == 0) {
            fs->cache[i].valid = 0;
        }
    }
}

static int read_head(struct vblk *dev, u64 *hash, bool *blank) {
    const u64 capacity = vblk_capacity(dev) * SECTOR_SIZE;
    const u64 size = capacity < HEAD_SIZE ? capacity : HEAD_SIZE;
    u8 raw[1024];
    u64 value = 0xCBF29CE484222325ull;
    int uniform = 1;
    u8 first = 0;
    for (u64 at = 0; at < size; at += sizeof(raw)) {
        const u32 len = size - at < sizeof(raw) ? (u32) (size - at) : (u32) sizeof(raw);
        const int result = vblk_read(dev, at / SECTOR_SIZE, raw, len);
        if (result != 0) {
            return result;
        }
        first = at == 0 ? raw[0] : first;
        for (u32 i = 0; i < len; i++) {
            uniform &= raw[i] == first;
            value = (value ^ raw[i]) * 0x100000001B3ull;
        }
    }
    *hash = value;
    *blank = uniform && (first == 0x00 || first == FLOPPY_FILL);
    return 0;
}

static bool is_same_medium(void) {
    for (;;) {
        vblk_media_changed(fs->dev);
        if (vblk_capacity(fs->dev) == 0) {
            return 0;
        }
        u8 raw[1024];
        int result = vblk_read(fs->dev, SUPERBLOCK_OFFSET / SECTOR_SIZE, raw, sizeof(raw));
        if (result == 0 && memcmp(raw, fs->disk_super, sizeof(raw)) != 0) {
            return 0;
        }
        if (result == 0 && fs->medium != FS_MEDIUM_EXT2) {
            u64 head;
            bool blank;
            result = read_head(fs->dev, &head, &blank);
            if (result == 0) {
                return head == fs->disk_head;
            }
        }
        if (result == VBLK_CHANGED) {
            continue;
        }
        return result == 0;
    }
}

static int transfer(const int write, const u64 sector, void *data, const u32 len) {
    for (;;) {
        if (fs->gone) {
            return FS_CHANGED;
        }
        const int result = write ? vblk_write(fs->dev, sector, data, len) : vblk_read(fs->dev, sector, data, len);
        if (result == 0) {
            fs->wrote |= write;
            return 0;
        }
        if (result != VBLK_CHANGED) {
            return FS_IO;
        }
        if (!is_same_medium()) {
            fs->gone = 1;
            return FS_CHANGED;
        }
        drop_cache();
    }
}

static int disk_read(const u32 block, void *dst) {
    return transfer(0, (u64) block * fs->sectors_per_block, dst, fs->block_size);
}

static int disk_write(const u32 block, const void *src) {
    return transfer(1, (u64) block * fs->sectors_per_block, (void *) src, fs->block_size);
}

static struct buf *cache_slot(const u32 block) {
    struct buf *victim = NULL;
    for (int i = 0; i < CACHE_SIZE; i++) {
        struct buf *b = &fs->cache[i];
        if (b->valid && b->block == block) {
            b->refs++;
            b->used = ++fs->cache_clock;
            return b;
        }
        if (b->refs == 0 && (victim == NULL || !b->valid || (victim->valid && b->used < victim->used))) {
            victim = b;
        }
    }
    if (victim == NULL) {
        kprintf("ext2: block cache exhausted\n");
        halt();
    }
    victim->valid = 0;
    victim->block = block;
    victim->refs = 1;
    victim->used = ++fs->cache_clock;
    return victim;
}

static struct buf *bread(const u32 block) {
    struct buf *b = cache_slot(block);
    if (!b->valid) {
        if (disk_read(block, b->data) != 0) {
            b->refs--;
            return NULL;
        }
        b->valid = 1;
    }
    return b;
}

static int bwrite(struct buf *b) {
    return disk_write(b->block, b->data);
}

static void brelse(struct buf *b) {
    b->refs--;
}

static int zero_block(const u32 block) {
    struct buf *b = cache_slot(block);
    memset(b->data, 0, fs->block_size);
    b->valid = 1;
    const int result = bwrite(b);
    brelse(b);
    return result;
}

static struct buf *bnew(const u32 block) {
    struct buf *b = cache_slot(block);
    memset(b->data, 0, fs->block_size);
    b->valid = 1;
    return b;
}

static int write_super(void) {
    fs->super_dirty = 1;
    return 0;
}

static int write_group(const u32 group) {
    fs->gdt_dirty[group / (fs->block_size / sizeof(struct group_desc))] = 1;
    return 0;
}

static int flush_metadata(void) {
    int result = 0;
    for (u32 i = 0; i < fs->gdt_blocks; i++) {
        if (fs->gdt_dirty[i]) {
            const int written = disk_write(SB->first_data_block + 1 + i, (u8 *) fs->gdt + (u64) i * fs->block_size);
            if (written == 0) {
                fs->gdt_dirty[i] = 0;
            } else {
                result = written;
            }
        }
    }
    if (fs->super_dirty) {
        SB->wtime = (u32) rtc_seconds();
        memset(fs->super.raw + LAST_MOUNTED_OFFSET, 0, LAST_MOUNTED_SIZE);
        snprintf((char *) fs->super.raw + LAST_MOUNTED_OFFSET, LAST_MOUNTED_SIZE, "openos2 %016lx %08x",
            fs->nonce, ++fs->flushes);
        const int written = transfer(1, SUPERBLOCK_OFFSET / SECTOR_SIZE, fs->super.raw, sizeof(fs->super.raw));
        if (written == 0) {
            fs->super_dirty = 0;
            memcpy(fs->disk_super, fs->super.raw, sizeof(fs->disk_super));
        } else {
            result = written;
        }
    }
    return result;
}

/* ---------------------------------------------------------------------------------------- */
/* Allocation */

static long take_bit(u8 *bitmap, const u32 count, const u32 start) {
    for (u32 n = 0; n < count; n++) {
        const u32 bit = (start + n) % count;
        if ((bitmap[bit / 8] & (1u << (bit % 8))) == 0) {
            bitmap[bit / 8] |= (u8) (1u << (bit % 8));
            return bit;
        }
    }
    return -1;
}

static u32 alloc_block(const int zero) {
    if (SB->free_blocks_count == 0) {
        return 0;
    }
    const u32 goal = fs->last_alloc + 1 < SB->blocks_count ? fs->last_alloc + 1 : SB->first_data_block;
    const u32 goal_group = (goal - SB->first_data_block) / SB->blocks_per_group;
    for (u32 n = 0; n < fs->groups; n++) {
        const u32 group = (goal_group + n) % fs->groups;
        struct group_desc *gd = &fs->gdt[group];
        if (gd->free_blocks_count == 0) {
            continue;
        }
        const u32 first = SB->first_data_block + group * SB->blocks_per_group;
        u32 count = SB->blocks_count - first;
        if (count > SB->blocks_per_group) {
            count = SB->blocks_per_group;
        }
        struct buf *b = bread(gd->block_bitmap);
        if (b == NULL) {
            return 0;
        }
        const long bit = take_bit(b->data, count, n == 0 ? goal - first : 0);
        if (bit < 0) {
            brelse(b);
            continue;
        }
        if (bwrite(b) != 0) {
            b->data[bit / 8] &= (u8) ~(1u << (bit % 8));
            brelse(b);
            return 0;
        }
        brelse(b);
        gd->free_blocks_count--;
        SB->free_blocks_count--;
        const u32 block = first + (u32) bit;
        if ((zero && zero_block(block) != 0) || write_group(group) != 0 || write_super() != 0) {
            return 0;
        }
        fs->last_alloc = block;
        return block;
    }
    return 0;
}

static bool is_valid_block(const u32 block) {
    return block >= SB->first_data_block && block < SB->blocks_count;
}

static bool is_valid_inode(const u32 number) {
    return number >= 1 && number <= SB->inodes_count;
}

static void free_block(const u32 block) {
    if (!is_valid_block(block)) {
        return;
    }
    const u32 group = (block - SB->first_data_block) / SB->blocks_per_group;
    const u32 bit = (block - SB->first_data_block) % SB->blocks_per_group;
    struct buf *b = bread(fs->gdt[group].block_bitmap);
    if (b == NULL) {
        return;
    }
    b->data[bit / 8] &= (u8) ~(1u << (bit % 8));
    if (bwrite(b) != 0) {
        b->data[bit / 8] |= (u8) (1u << (bit % 8));
        brelse(b);
        return;
    }
    brelse(b);
    fs->gdt[group].free_blocks_count++;
    SB->free_blocks_count++;
    write_group(group);
    write_super();
}

static int read_inode(const u32 number, struct inode *inode) {
    if (!is_valid_inode(number)) {
        return FS_IO;
    }
    const u32 group = (number - 1) / SB->inodes_per_group;
    const u32 offset = ((number - 1) % SB->inodes_per_group) * fs->inode_size;
    struct buf *b = bread(fs->gdt[group].inode_table + offset / fs->block_size);
    if (b == NULL) {
        return FS_IO;
    }
    inode->number = number;
    memcpy(&inode->d, b->data + offset % fs->block_size, sizeof(inode->d));
    brelse(b);
    return 0;
}

static int write_inode(const struct inode *inode) {
    if (!is_valid_inode(inode->number)) {
        return FS_IO;
    }
    const u32 group = (inode->number - 1) / SB->inodes_per_group;
    const u32 offset = ((inode->number - 1) % SB->inodes_per_group) * fs->inode_size;
    struct buf *b = bread(fs->gdt[group].inode_table + offset / fs->block_size);
    if (b == NULL) {
        return FS_IO;
    }
    memcpy(b->data + offset % fs->block_size, &inode->d, sizeof(inode->d));
    const int result = bwrite(b);
    brelse(b);
    return result;
}

static int alloc_inode(const u32 parent, const int is_dir, struct inode *inode) {
    if (SB->free_inodes_count == 0) {
        return FS_NO_SPACE;
    }
    const u32 start = (parent - 1) / SB->inodes_per_group;
    for (u32 n = 0; n < fs->groups; n++) {
        const u32 group = (start + n) % fs->groups;
        struct group_desc *gd = &fs->gdt[group];
        if (gd->free_inodes_count == 0) {
            continue;
        }
        struct buf *b = bread(gd->inode_bitmap);
        if (b == NULL) {
            return FS_IO;
        }
        const u32 first_bit = group == 0 ? fs->first_ino - 1 : 0;
        const long bit = take_bit(b->data, SB->inodes_per_group, first_bit);
        if (bit >= 0 && (u32) bit < first_bit) {
            b->data[bit / 8] &= (u8) ~(1u << (bit % 8));
            brelse(b);
            continue;
        }
        if (bit < 0) {
            brelse(b);
            continue;
        }
        if (bwrite(b) != 0) {
            b->data[bit / 8] &= (u8) ~(1u << (bit % 8));
            brelse(b);
            return FS_IO;
        }
        brelse(b);
        gd->free_inodes_count--;
        if (is_dir) {
            gd->used_dirs_count++;
        }
        SB->free_inodes_count--;
        if (write_group(group) != 0 || write_super() != 0) {
            return FS_IO;
        }

        const u32 number = group * SB->inodes_per_group + (u32) bit + 1;
        const u32 offset = (u32) bit * fs->inode_size;
        struct buf *table = bread(gd->inode_table + offset / fs->block_size);
        if (table == NULL) {
            return FS_IO;
        }
        memset(table->data + offset % fs->block_size, 0, fs->inode_size);
        bwrite(table);
        brelse(table);

        memset(inode, 0, sizeof(*inode));
        inode->number = number;
        const u32 now = (u32) rtc_seconds();
        inode->d.atime = inode->d.ctime = inode->d.mtime = now;
        return 0;
    }
    return FS_NO_SPACE;
}

static void free_inode(struct inode *inode) {
    const int is_dir = (inode->d.mode & S_IFMT) == S_IFDIR;
    inode->d.dtime = (u32) rtc_seconds();
    inode->d.links_count = 0;
    if (write_inode(inode) != 0) {
        return;
    }

    const u32 group = (inode->number - 1) / SB->inodes_per_group;
    const u32 bit = (inode->number - 1) % SB->inodes_per_group;
    struct buf *b = bread(fs->gdt[group].inode_bitmap);
    if (b == NULL) {
        return;
    }
    b->data[bit / 8] &= (u8) ~(1u << (bit % 8));
    bwrite(b);
    brelse(b);
    fs->gdt[group].free_inodes_count++;
    if (is_dir) {
        fs->gdt[group].used_dirs_count--;
    }
    SB->free_inodes_count++;
    write_group(group);
    write_super();
}

/* ---------------------------------------------------------------------------------------- */
/* Block mapping */

static u32 bmap(struct inode *inode, u32 index, const int allocate, int *error, int *fresh) {
    u32 *slot;
    int levels;
    const u32 ppb = fs->pointers_per_block;
    if (index < 12) {
        slot = &inode->d.block[index];
        levels = 0;
    } else if ((index -= 12) < ppb) {
        slot = &inode->d.block[12];
        levels = 1;
    } else if ((index -= ppb) < ppb * ppb) {
        slot = &inode->d.block[13];
        levels = 2;
    } else {
        index -= ppb * ppb;
        slot = &inode->d.block[14];
        levels = 3;
    }

    u32 block = *slot;
    if (block != 0 && !is_valid_block(block)) {
        *error = FS_IO;
        return 0;
    }
    if (block == 0) {
        if (!allocate || (block = alloc_block(levels > 0)) == 0) {
            if (allocate) {
                *error = FS_NO_SPACE;
            }
            return 0;
        }
        *slot = block;
        inode->d.blocks += fs->sectors_per_block;
        if (levels == 0 && fresh != NULL) {
            *fresh = 1;
        }
    }

    for (int level = levels; level > 0; level--) {
        u32 divisor = 1;
        for (int i = 1; i < level; i++) {
            divisor *= ppb;
        }
        struct buf *b = bread(block);
        if (b == NULL) {
            *error = FS_IO;
            return 0;
        }
        u32 *entries = (u32 *) b->data;
        const u32 entry = (index / divisor) % ppb;
        u32 next = entries[entry];
        if (next == 0 && allocate) {
            next = alloc_block(level > 1);
            if (next == 0) {
                *error = FS_NO_SPACE;
                brelse(b);
                return 0;
            }
            entries[entry] = next;
            inode->d.blocks += fs->sectors_per_block;
            if (level == 1 && fresh != NULL) {
                *fresh = 1;
            }
            if (bwrite(b) != 0) {
                *error = FS_IO;
                brelse(b);
                return 0;
            }
        }
        brelse(b);
        block = next;
        if (block == 0) {
            return 0;
        }
        if (!is_valid_block(block)) {
            *error = FS_IO;
            return 0;
        }
    }
    return block;
}

static void free_tree(struct inode *inode, const u32 block, const int level) {
    if (block == 0 || !is_valid_block(block)) {
        return;
    }
    if (level > 0) {
        struct buf *b = bread(block);
        if (b != NULL) {
            const u32 *entries = (const u32 *) b->data;
            for (u32 i = 0; i < fs->pointers_per_block; i++) {
                free_tree(inode, entries[i], level - 1);
            }
            brelse(b);
        }
    }
    free_block(block);
    inode->d.blocks -= fs->sectors_per_block;
}

static bool has_blocks(const struct inode *inode);

static int truncate(struct inode *inode) {
    struct inode old = *inode;
    memset(inode->d.block, 0, sizeof(inode->d.block));
    inode->d.size = 0;
    inode->d.size_high = 0;
    inode->d.blocks = inode->d.file_acl != 0 ? fs->sectors_per_block : 0;
    const int result = write_inode(inode);
    if (result != 0) {
        *inode = old;
        return result;
    }
    if (has_blocks(&old)) {
        for (int i = 0; i < 12; i++) {
            free_tree(&old, old.d.block[i], 0);
        }
        free_tree(&old, old.d.block[12], 1);
        free_tree(&old, old.d.block[13], 2);
        free_tree(&old, old.d.block[14], 3);
    }
    return 0;
}

/* ---------------------------------------------------------------------------------------- */
/* Directories */

struct location {
    u32 block_index;
    u32 offset;
    long prev_offset; /* -1 if the entry starts its block */
    u32 inode;
    u8 file_type;
};

static u32 entry_size(const u32 name_len) {
    return (8 + name_len + 3) & ~3u;
}

static bool is_valid_entry(const struct dirent *e, const u32 offset) {
    return e->rec_len >= 8 && (e->rec_len & 3) == 0 && offset + e->rec_len <= fs->block_size
        && 8u + e->name_len <= e->rec_len;
}

static u32 block_count(const struct inode *inode) {
    return (inode->d.size + fs->block_size - 1) / fs->block_size;
}

static bool is_dir(const struct inode *inode) {
    return (inode->d.mode & S_IFMT) == S_IFDIR;
}

static bool has_blocks(const struct inode *inode) {
    const u32 type = inode->d.mode & S_IFMT;
    return type == S_IFREG || type == S_IFDIR || (type == S_IFLNK && inode->d.size >= FAST_SYMLINK_MAX);
}

static int dir_find(struct inode *dir, const char *name, const u32 len, struct location *loc) {
    const u32 count = block_count(dir);
    for (u32 i = 0; i < count; i++) {
        int error = 0;
        const u32 block = bmap(dir, i, 0, &error, NULL);
        if (block == 0) {
            continue;
        }
        struct buf *b = bread(block);
        if (b == NULL) {
            return FS_IO;
        }
        long prev = -1;
        for (u32 offset = 0; offset + 8 <= fs->block_size;) {
            const struct dirent *e = (const struct dirent *) (b->data + offset);
            if (!is_valid_entry(e, offset)) {
                brelse(b);
                return FS_IO;
            }
            if (e->inode != 0 && e->name_len == len && memcmp(e->name, name, len) == 0) {
                loc->block_index = i;
                loc->offset = offset;
                loc->prev_offset = prev;
                loc->inode = e->inode;
                loc->file_type = e->file_type;
                brelse(b);
                return 0;
            }
            prev = offset;
            offset += e->rec_len;
        }
        brelse(b);
    }
    return FS_NOT_FOUND;
}

static void drop_index(struct inode *dir) {
    dir->d.flags &= ~INODE_INDEX_FL;
}

static void fill_entry(struct dirent *e, const char *name, const u32 len, const u32 inode, const int type) {
    e->inode = inode;
    e->name_len = (u8) len;
    e->file_type = fs->filetype ? (u8) type : 0;
    memcpy(e->name, name, len);
}

struct slot {
    u32 index;
    u32 offset;
    u32 used;
};

static int dir_reserve(struct inode *dir, const u32 len, struct slot *slot) {
    const u32 need = entry_size(len);
    const u32 count = block_count(dir);
    if (dir->d.flags & INODE_INDEX_FL) {
        drop_index(dir);
        const int result = write_inode(dir);
        if (result != 0) {
            return result;
        }
    }
    for (u32 i = 0; i < count; i++) {
        int error = 0;
        const u32 block = bmap(dir, i, 0, &error, NULL);
        if (block == 0) {
            continue;
        }
        struct buf *b = bread(block);
        if (b == NULL) {
            return FS_IO;
        }
        for (u32 offset = 0; offset + 8 <= fs->block_size;) {
            const struct dirent *e = (const struct dirent *) (b->data + offset);
            if (!is_valid_entry(e, offset)) {
                brelse(b);
                return FS_IO;
            }
            const u32 used = e->inode != 0 ? entry_size(e->name_len) : 0;
            if (e->rec_len - used >= need) {
                *slot = (struct slot) {i, offset, used};
                brelse(b);
                return 0;
            }
            offset += e->rec_len;
        }
        brelse(b);
    }

    int error = 0;
    const u32 block = bmap(dir, count, 1, &error, NULL);
    if (block == 0) {
        write_inode(dir);
        return error != 0 ? error : FS_NO_SPACE;
    }
    struct buf *b = bnew(block);
    ((struct dirent *) b->data)->rec_len = (u16) fs->block_size;
    int result = bwrite(b);
    brelse(b);
    if (result == 0) {
        dir->d.size += fs->block_size;
        result = write_inode(dir);
    }
    *slot = (struct slot) {count, 0, 0};
    return result;
}

static int dir_put(struct inode *dir, const struct slot *slot, const char *name, const u32 len, const u32 inode, const int type) {
    int error = 0;
    const u32 block = bmap(dir, slot->index, 0, &error, NULL);
    struct buf *b = block != 0 ? bread(block) : NULL;
    if (b == NULL) {
        return FS_IO;
    }
    struct dirent *e = (struct dirent *) (b->data + slot->offset);
    if (slot->used != 0) {
        struct dirent *next = (struct dirent *) (b->data + slot->offset + slot->used);
        next->rec_len = (u16) (e->rec_len - slot->used);
        e->rec_len = (u16) slot->used;
        e = next;
    }
    fill_entry(e, name, len, inode, type);
    const int result = bwrite(b);
    brelse(b);
    return result;
}

static int dir_remove(struct inode *dir, const struct location *loc) {
    int error = 0;
    const u32 block = bmap(dir, loc->block_index, 0, &error, NULL);
    struct buf *b = block != 0 ? bread(block) : NULL;
    if (b == NULL) {
        return FS_IO;
    }
    struct dirent *e = (struct dirent *) (b->data + loc->offset);
    if (loc->prev_offset >= 0) {
        ((struct dirent *) (b->data + loc->prev_offset))->rec_len += e->rec_len;
    } else {
        e->inode = 0;
    }
    const int result = bwrite(b);
    brelse(b);
    drop_index(dir);
    dir->d.mtime = dir->d.ctime = (u32) rtc_seconds();
    return result != 0 ? result : write_inode(dir);
}

static int dir_set_entry(struct inode *dir, const struct location *loc, const u32 inode, const int type) {
    int error = 0;
    const u32 block = bmap(dir, loc->block_index, 0, &error, NULL);
    struct buf *b = block != 0 ? bread(block) : NULL;
    if (b == NULL) {
        return FS_IO;
    }
    struct dirent *e = (struct dirent *) (b->data + loc->offset);
    e->inode = inode;
    e->file_type = fs->filetype ? (u8) type : 0;
    const int result = bwrite(b);
    brelse(b);
    return result;
}

static int dir_set_parent(struct inode *dir, const u32 parent) {
    struct location loc;
    const int result = dir_find(dir, "..", 2, &loc);
    if (result != 0) {
        return result;
    }
    int error = 0;
    const u32 block = bmap(dir, loc.block_index, 0, &error, NULL);
    struct buf *b = block != 0 ? bread(block) : NULL;
    if (b == NULL) {
        return FS_IO;
    }
    ((struct dirent *) (b->data + loc.offset))->inode = parent;
    const int written = bwrite(b);
    brelse(b);
    return written;
}

static bool is_dir_empty(struct inode *dir) {
    const u32 count = block_count(dir);
    for (u32 i = 0; i < count; i++) {
        int error = 0;
        const u32 block = bmap(dir, i, 0, &error, NULL);
        if (block == 0) {
            continue;
        }
        struct buf *b = bread(block);
        if (b == NULL) {
            return false;
        }
        for (u32 offset = 0; offset + 8 <= fs->block_size;) {
            const struct dirent *e = (const struct dirent *) (b->data + offset);
            if (!is_valid_entry(e, offset)) {
                break;
            }
            const int dot = e->name_len == 1 && e->name[0] == '.';
            const int dotdot = e->name_len == 2 && e->name[0] == '.' && e->name[1] == '.';
            if (e->inode != 0 && !dot && !dotdot) {
                brelse(b);
                return false;
            }
            offset += e->rec_len;
        }
        brelse(b);
    }
    return true;
}

/* ---------------------------------------------------------------------------------------- */
/* Paths */

static int lookup(const char *path, struct inode *inode, const char **leaf, u32 *leaf_len) {
    int result = read_inode(ROOT_INODE, inode);
    if (result != 0) {
        return result;
    }
    const char *p = path;
    for (;;) {
        const char *separator = p;
        while (*p == '/') {
            p++;
        }
        if (*p == 0) {
            if (leaf != NULL) {
                return FS_INVALID;
            }
            if (p != separator && p != path && !is_dir(inode)) {
                return FS_NOT_DIR;
            }
            return 0;
        }
        const char *name = p;
        while (*p != 0 && *p != '/') {
            p++;
        }
        const u32 len = (u32) (p - name);
        const char *rest = p;
        while (*rest == '/') {
            rest++;
        }
        if (len > 255) {
            return FS_INVALID;
        }
        if (!is_dir(inode)) {
            return FS_NOT_DIR;
        }
        if (leaf != NULL && *rest == 0) {
            if ((len == 1 && name[0] == '.') || (len == 2 && name[0] == '.' && name[1] == '.')) {
                return FS_INVALID;
            }
            *leaf = name;
            *leaf_len = len;
            return 0;
        }
        struct location loc;
        result = dir_find(inode, name, len, &loc);
        if (result != 0) {
            return result;
        }
        result = read_inode(loc.inode, inode);
        if (result != 0) {
            return result;
        }
    }
}

/* ---------------------------------------------------------------------------------------- */
/* Mounts */

static void free_buffers(struct mount *m) {
    for (int i = 0; i < CACHE_SIZE; i++) {
        free(m->cache[i].data);
        m->cache[i].data = NULL;
    }
    free(m->gdt);
    free(m->gdt_dirty);
    m->gdt = NULL;
    m->gdt_dirty = NULL;
}

static void unmount(struct mount *m) {
    for (int i = 0; i < MAX_FILES; i++) {
        if (files[i].open && files[i].mount == m) {
            files[i].mount = NULL;
        }
    }
    for (int i = 0; i < MAX_RAW; i++) {
        if (raws[i].open && raws[i].mount == m) {
            raws[i].mount = NULL;
        }
    }
    free_buffers(m);
    mounts[m->device] = NULL;
    if (root == m) {
        root = NULL;
    }
    free(m);
}

static int check_geometry(void) {
    const u32 bits = 8 * fs->block_size;
    if (SB->blocks_per_group == 0 || SB->blocks_per_group > bits || SB->inodes_per_group == 0
        || SB->inodes_per_group > bits || SB->first_data_block >= SB->blocks_count
        || fs->inode_size < 128 || fs->inode_size > fs->block_size || (fs->inode_size & (fs->inode_size - 1)) != 0
        || (u64) SB->blocks_count * fs->sectors_per_block > vblk_capacity(fs->dev)) {
        return FS_INVALID;
    }
    fs->groups = (SB->blocks_count - SB->first_data_block + SB->blocks_per_group - 1) / SB->blocks_per_group;
    if (SB->inodes_count > (u64) fs->groups * SB->inodes_per_group) {
        return FS_INVALID;
    }
    return 0;
}

static int check_groups(void) {
    const u32 table_blocks = (SB->inodes_per_group * fs->inode_size + fs->block_size - 1) / fs->block_size;
    for (u32 group = 0; group < fs->groups; group++) {
        const struct group_desc *gd = &fs->gdt[group];
        if (!is_valid_block(gd->block_bitmap) || !is_valid_block(gd->inode_bitmap) || !is_valid_block(gd->inode_table)
            || (u64) gd->inode_table + table_blocks > SB->blocks_count) {
            return FS_INVALID;
        }
    }
    return 0;
}

static int read_gdt(void) {
    for (u32 i = 0; i < fs->gdt_blocks; i++) {
        const int result = disk_read(SB->first_data_block + 1 + i, (u8 *) fs->gdt + (u64) i * fs->block_size);
        if (result != 0) {
            return result;
        }
    }
    return check_groups();
}

static int read_mount(struct vblk *dev) {
    fs->dev = dev;
    vblk_media_changed(dev);
    if (vblk_capacity(dev) == 0) {
        return FS_NOT_FOUND;
    }
    if (transfer(0, SUPERBLOCK_OFFSET / SECTOR_SIZE, fs->super.raw, sizeof(fs->super.raw)) != 0) {
        return FS_IO;
    }
    memcpy(fs->disk_super, fs->super.raw, sizeof(fs->disk_super));
    fs->medium = SB->magic == EXT2_MAGIC ? FS_MEDIUM_UNSUPPORTED : FS_MEDIUM_UNKNOWN;
    if (SB->magic != EXT2_MAGIC || SB->log_block_size > 2 || SB->rev_level > 1) {
        return FS_INVALID;
    }
    const u32 incompat = SB->rev_level >= 1 ? SB->feature_incompat : 0;
    const u32 ro_compat = SB->rev_level >= 1 ? SB->feature_ro_compat : 0;
    if (incompat & ~INCOMPAT_FILETYPE) {
        return FS_INVALID;
    }
    fs->writable = !vblk_read_only(dev) && (ro_compat & ~(RO_COMPAT_SPARSE_SUPER | RO_COMPAT_LARGE_FILE)) == 0;
    fs->filetype = (incompat & INCOMPAT_FILETYPE) != 0;

    fs->block_size = 1024u << SB->log_block_size;
    fs->sectors_per_block = fs->block_size / SECTOR_SIZE;
    fs->pointers_per_block = fs->block_size / 4;
    fs->inode_size = SB->rev_level >= 1 ? SB->inode_size : 128;
    fs->first_ino = SB->rev_level >= 1 ? SB->first_ino : 11;
    if (check_geometry() != 0) {
        return FS_INVALID;
    }
    const u32 per_block = fs->block_size / sizeof(struct group_desc);
    fs->gdt_blocks = (fs->groups + per_block - 1) / per_block;
    fs->gdt = malloc((size_t) fs->gdt_blocks * fs->block_size);
    fs->gdt_dirty = calloc(fs->gdt_blocks, 1);
    if (fs->gdt == NULL || fs->gdt_dirty == NULL) {
        return FS_NO_SPACE;
    }
    const int result = read_gdt();
    if (result != 0) {
        return result;
    }
    fs->medium = FS_MEDIUM_EXT2;
    for (int i = 0; i < CACHE_SIZE; i++) {
        fs->cache[i].data = malloc(fs->block_size);
        if (fs->cache[i].data == NULL) {
            return FS_NO_SPACE;
        }
    }
    fs->last_alloc = SB->first_data_block;
    return 0;
}

static int stamp(void) {
    u64 previous = 0;
    for (size_t i = 0; i < sizeof(fs->super.raw); i++) {
        previous = (previous ^ fs->super.raw[i]) * 0x100000001B3ull;
    }
    fs->nonce = mix64(entropy() ^ mix64(previous + (u64) fs->device));
    SB->mnt_count++;
    SB->mtime = (u32) rtc_seconds();
    SB->state &= (u16) ~STATE_CLEAN;
    fs->super_dirty = 1;
    return flush_metadata();
}

static bool is_valid_label(const char *label, const size_t len) {
    if (len == 0 || len > FS_LABEL_LENGTH || strcmp(label, ".") == 0 || strcmp(label, "..") == 0) {
        return 0;
    }
    int digits = 1;
    for (size_t i = 0; i < len; i++) {
        const char c = label[i];
        if (!((c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '.' || c == '_' || c == '-')) {
            return 0;
        }
        digits &= c >= '0' && c <= '9';
    }
    return !digits;
}

static void read_label(const struct mount *m, char *label) {
    memset(label, 0, FS_LABEL_LENGTH + 1);
    if (m->medium == FS_MEDIUM_EXT2 || m->medium == FS_MEDIUM_UNSUPPORTED) {
        memcpy(label, m->super.raw + VOLUME_NAME_OFFSET, FS_LABEL_LENGTH);
    }
}

static bool is_name_taken(const char *name, struct mount *self) {
    char path[6 + FS_LABEL_LENGTH];
    snprintf(path, sizeof(path), "/mnt/%s", name);
    const char *rest;
    if (p9_resolve(path, &rest) != NULL && *rest == 0) {
        return 1;
    }
    if (root != NULL && root != self) {
        struct inode inode;
        fs = root;
        const int found = lookup(path + 1, &inode, NULL, NULL) == 0;
        fs = self;
        if (found) {
            return 1;
        }
    }
    for (int i = 0; i < MAX_VIRTIO; i++) {
        if (mounts[i] != NULL && mounts[i] != self && strcmp(mounts[i]->path, path) == 0) {
            return 1;
        }
    }
    return 0;
}

static void name_mount(struct mount *m) {
    char label[FS_LABEL_LENGTH + 1];
    read_label(m, label);
    if (m->medium == FS_MEDIUM_EXT2 && is_valid_label(label, strlen(label)) && !is_name_taken(label, m)) {
        snprintf(m->path, sizeof(m->path), "/mnt/%s", label);
    } else {
        snprintf(m->path, sizeof(m->path), "/mnt/%d", m->device);
    }
}

static int read_unusable(struct mount *m) {
    free_buffers(m);
    m->writable = !vblk_read_only(m->dev);
    bool blank;
    if (read_head(m->dev, &m->disk_head, &blank) != 0) {
        return FS_IO;
    }
    if (blank) {
        m->medium = FS_MEDIUM_BLANK;
    }
    return 0;
}

static struct mount *mount_device(const int device) {
    struct mount *m = calloc(1, sizeof(struct mount));
    if (m == NULL) {
        return NULL;
    }
    m->device = device;
    m->serial = ++serials;
    fs = m;
    mounts[device] = m;
    int result = read_mount(vblk_get(device));
    if (result == FS_INVALID) {
        result = read_unusable(m);
    } else if (result == 0 && m->writable) {
        result = stamp();
    }
    if (result != 0) {
        unmount(m);
        return NULL;
    }
    name_mount(m);
    return m;
}

static bool has_boot_image(void) {
    struct inode inode;
    return fs->medium == FS_MEDIUM_EXT2 && lookup("boot/Image", &inode, NULL, NULL) == 0;
}

static void make_root(struct mount *m) {
    root = m;
    strcpy(m->path, "/");
}

void ext2_mount_all(void) {
    for (int i = 0; i < vblk_count(); i++) {
        struct mount *m = mount_device(i);
        if (m != NULL && boot_device < 0 && has_boot_image()) {
            boot_device = i;
            make_root(m);
        }
    }
}

static int mark_clean(void) {
    if (fs->medium != FS_MEDIUM_EXT2 || !fs->writable || fs->gone) {
        return 0;
    }
    SB->state |= STATE_CLEAN;
    fs->super_dirty = 1;
    return flush_metadata();
}

void ext2_unmount_all(void) {
    for (int i = 0; i < vblk_count(); i++) {
        fs = mounts[i];
        if (fs != NULL) {
            mark_clean();
        }
    }
}

static int enter_mount_point(const char *path, u32 serial);
static bool has_open_files(const struct mount *m);
static void report(const char *old_path, int device);

int ext2_unmount(const char *path, const u32 serial) {
    const int result = enter_mount_point(path, serial);
    if (result != 0) {
        return result;
    }
    if (fs == root) {
        return FS_IS_ROOT;
    }
    if (has_open_files(fs) || fs->raw_readers > 0 || fs->detached) {
        return FS_BUSY;
    }
    const int cleaned = mark_clean();
    if (cleaned != 0) {
        return cleaned;
    }
    char old_path[sizeof(fs->path)];
    strcpy(old_path, fs->path);
    const int device = fs->device;
    unmount(fs);
    report(old_path, device);
    return 0;
}

static void remount(const int device) {
    struct mount *m = mount_device(device);
    if (m != NULL && device == boot_device && has_boot_image()) {
        make_root(m);
    }
}

static void report(const char *old_path, const int device) {
    const char *new_path = mounts[device] != NULL ? mounts[device]->path : NULL;
    if (old_path[0] != 0 && new_path != NULL && strcmp(old_path, new_path) == 0) {
        fs_medium_event(new_path, "changed");
    } else {
        if (old_path[0] != 0) {
            fs_medium_event(old_path, "removed");
        }
        if (new_path != NULL) {
            fs_medium_event(new_path, "added");
        }
    }
}

static void check_media(void) {
    char old_paths[MAX_VIRTIO][sizeof(mounts[0]->path)];
    int changed[MAX_VIRTIO] = {0};
    for (int i = 0; i < vblk_count(); i++) {
        struct vblk *dev = vblk_get(i);
        struct mount *m = mounts[i];
        if (m != NULL && !m->gone && !vblk_change_pending(dev)) {
            continue;
        }
        if (m == NULL && !vblk_change_pending(dev)) {
            continue;
        }
        if (m != NULL && !m->gone) {
            fs = m;
            int dirty = fs->super_dirty;
            for (u32 j = 0; j < fs->gdt_blocks; j++) {
                dirty |= fs->gdt_dirty[j];
            }
            if (is_same_medium()) {
                if (fs->medium != FS_MEDIUM_EXT2) {
                    continue;
                }
                drop_cache();
                if (dirty || read_gdt() == 0) {
                    continue;
                }
            }
        }
        changed[i] = 1;
        old_paths[i][0] = 0;
        if (m != NULL) {
            strcpy(old_paths[i], m->path);
            unmount(m);
        }
    }
    for (int i = 0; i < vblk_count(); i++) {
        if (changed[i]) {
            remount(i);
            report(old_paths[i], i);
        }
    }
}

void ext2_poll_media(void) {
    check_media();
}

void ext2_mounts(const fs_mounts_fn fn, void *ctx) {
    check_media();
    for (int i = 0; i < vblk_count(); i++) {
        const struct mount *m = mounts[i];
        if (m == NULL) {
            continue;
        }
        char label[FS_LABEL_LENGTH + 1];
        read_label(m, label);
        struct fs_mount mount = {.path = m->path, .label = label, .index = i, .medium = m->medium, .serial = m->serial,
            .writable = m->writable};
        if (m->medium == FS_MEDIUM_EXT2) {
            mount.total = (u64) m->super.sb.blocks_count * m->block_size;
            mount.free = (u64) m->super.sb.free_blocks_count * m->block_size;
        } else {
            mount.total = vblk_capacity(m->dev) * SECTOR_SIZE;
        }
        fn(ctx, &mount);
    }
}

void ext2_mount_names(struct fs_listing *out) {
    check_media();
    for (int i = 0; i < vblk_count(); i++) {
        if (mounts[i] != NULL && mounts[i] != root) {
            const char *name = mounts[i]->path + 5;
            fs_listing_add_unique(out, name, strlen(name), 1);
        }
    }
}

/* ---------------------------------------------------------------------------------------- */
/* Querying */

static const char *select_mount(const char *path) {
    fs = root;
    const char *p = path;
    while (*p == '/') {
        p++;
    }
    if (strncmp(p, "mnt/", 4) != 0) {
        return p;
    }
    const char *name = p + 4;
    const char *end = name;
    int digits = 1;
    while (*end != 0 && *end != '/') {
        digits &= *end >= '0' && *end <= '9';
        end++;
    }
    for (int i = 0; i < MAX_VIRTIO; i++) {
        const struct mount *m = mounts[i];
        if (m != NULL && m != root && strncmp(m->path + 5, name, (size_t) (end - name)) == 0
            && m->path[5 + (end - name)] == 0) {
            fs = mounts[i];
            return end;
        }
    }
    if (digits && end != name) {
        fs = NULL;
        return end;
    }
    return p;
}

static const char *enter(const char *path) {
    check_media();
    const char *rest = select_mount(path);
    if (fs != NULL) {
        fs->wrote = 0;
    }
    return rest;
}

static int unusable(void) {
    if (fs == NULL) {
        return FS_NOT_FOUND;
    }
    if (fs->detached) {
        return FS_BUSY;
    }
    return fs->medium == FS_MEDIUM_EXT2 ? 0 : fs->medium == FS_MEDIUM_BLANK ? FS_UNFORMATTED : FS_UNSUPPORTED;
}

int ext2_info(const char *path, u64 *total, u64 *free) {
    enter(path);
    const int refused = unusable();
    if (refused != 0) {
        return refused;
    }
    *total = (u64) SB->blocks_count * fs->block_size;
    *free = (u64) SB->free_blocks_count * fs->block_size;
    return 0;
}

int ext2_stat(const char *path, struct fs_stat *st) {
    path = enter(path);
    const int refused = unusable();
    if (refused != 0) {
        return refused;
    }
    struct inode inode;
    const int result = lookup(path, &inode, NULL, NULL);
    if (result != 0) {
        return result;
    }
    st->size = inode.d.size;
    st->mtime = inode.d.mtime;
    st->is_dir = is_dir(&inode);
    return 0;
}

int ext2_list(const char *path, struct fs_listing *out) {
    path = enter(path);
    const int refused = unusable();
    if (refused != 0) {
        return refused;
    }
    struct inode dir;
    const int result = lookup(path, &dir, NULL, NULL);
    if (result != 0) {
        return result;
    }
    if (!is_dir(&dir)) {
        return FS_NOT_DIR;
    }
    const u32 count = block_count(&dir);
    for (u32 i = 0; i < count; i++) {
        int error = 0;
        const u32 block = bmap(&dir, i, 0, &error, NULL);
        if (block == 0) {
            continue;
        }
        struct buf *b = bread(block);
        if (b == NULL) {
            return FS_IO;
        }
        for (u32 offset = 0; offset + 8 <= fs->block_size;) {
            const struct dirent *e = (const struct dirent *) (b->data + offset);
            if (!is_valid_entry(e, offset)) {
                break;
            }
            const int dot = e->name_len == 1 && e->name[0] == '.';
            const int dotdot = e->name_len == 2 && e->name[0] == '.' && e->name[1] == '.';
            if (e->inode != 0 && !dot && !dotdot) {
                int entry_is_dir = e->file_type == FT_DIR;
                if (!fs->filetype) {
                    struct inode child;
                    entry_is_dir = read_inode(e->inode, &child) == 0 && is_dir(&child);
                }
                if (fs_listing_add(out, e->name, e->name_len, entry_is_dir) != 0) {
                    brelse(b);
                    return FS_NO_SPACE;
                }
            }
            offset += e->rec_len;
        }
        brelse(b);
    }
    return 0;
}

static int enter_mount_point(const char *path, const u32 serial) {
    const char *rest = enter(path);
    if (fs == NULL) {
        return FS_NOT_FOUND;
    }
    while (*rest == '/') {
        rest++;
    }
    if (*rest != 0) {
        return fs == root && strncmp(rest, "mnt/", 4) == 0 && strchr(rest + 4, '/') == NULL ? FS_NOT_FOUND : FS_INVALID;
    }
    return serial != 0 && serial != fs->serial ? FS_CHANGED : 0;
}

int ext2_label(const char *path, char *label) {
    const int result = enter_mount_point(path, 0);
    if (result != 0) {
        return result;
    }
    const int refused = unusable();
    if (refused != 0) {
        return refused;
    }
    read_label(fs, label);
    return 0;
}

/* ---------------------------------------------------------------------------------------- */
/* Opening */

static int leave(const int result) {
    if (result == FS_IO && fs->wrote && !fs->gone) {
        SB->state |= STATE_ERROR;
        fs->super_dirty = 1;
    }
    const int flushed = flush_metadata();
    return result != 0 ? result : flushed;
}

static int unwritable(void) {
    const int refused = unusable();
    return refused != 0 ? refused : fs->raw_readers > 0 ? FS_BUSY : 0;
}

static int create(const char *path, const int dir, struct inode *out) {
    struct inode parent;
    const char *name;
    u32 len;
    int result = lookup(path, &parent, &name, &len);
    if (result != 0) {
        return result;
    }
    if (!is_dir(&parent)) {
        return FS_NOT_DIR;
    }
    struct location loc;
    result = dir_find(&parent, name, len, &loc);
    if (result != FS_NOT_FOUND) {
        return result == 0 ? FS_EXISTS : result;
    }
    if (!fs->writable) {
        return FS_READ_ONLY;
    }
    struct slot slot;
    result = dir_reserve(&parent, len, &slot);
    if (result != 0) {
        return result;
    }

    struct inode inode;
    result = alloc_inode(parent.number, dir, &inode);
    if (result != 0) {
        return result;
    }
    if (dir) {
        inode.d.mode = S_IFDIR | 0755;
        inode.d.links_count = 2;
        int error = 0;
        const u32 block = bmap(&inode, 0, 1, &error, NULL);
        if (block == 0) {
            free_inode(&inode);
            return error != 0 ? error : FS_IO;
        }
        struct buf *b = bnew(block);
        struct dirent *dot = (struct dirent *) b->data;
        dot->rec_len = 12;
        fill_entry(dot, ".", 1, inode.number, FT_DIR);
        struct dirent *dotdot = (struct dirent *) (b->data + 12);
        dotdot->rec_len = (u16) (fs->block_size - 12);
        fill_entry(dotdot, "..", 2, parent.number, FT_DIR);
        result = bwrite(b);
        brelse(b);
        if (result != 0) {
            return result;
        }
        inode.d.size = fs->block_size;
    } else {
        inode.d.mode = S_IFREG | 0644;
        inode.d.links_count = 1;
    }
    result = write_inode(&inode);
    if (result == 0 && dir) {
        parent.d.links_count++;
        result = write_inode(&parent);
    }
    if (result == 0) {
        result = dir_put(&parent, &slot, name, len, inode.number, dir ? FT_DIR : FT_REG);
    }
    if (result != 0) {
        return result;
    }
    if (out != NULL) {
        *out = inode;
    }
    return 0;
}

static int open_file(const char *path, const int flags) {
    int fd = -1;
    for (int i = 0; i < MAX_FILES; i++) {
        if (!files[i].open) {
            fd = i;
            break;
        }
    }
    if (fd < 0) {
        return FS_TOO_MANY;
    }
    if ((flags & O_WRITE) && !fs->writable) {
        return FS_READ_ONLY;
    }
    const size_t length = strlen(path);
    if (length > 0 && path[length - 1] == '/') {
        return FS_IS_DIR;
    }

    struct inode inode;
    int result = lookup(path, &inode, NULL, NULL);
    if (result == FS_NOT_FOUND && (flags & O_CREATE)) {
        result = create(path, 0, &inode);
    } else if (result == 0 && is_dir(&inode)) {
        result = FS_IS_DIR;
    } else if (result == 0 && (inode.d.mode & S_IFMT) != S_IFREG) {
        result = FS_INVALID;
    } else if (result == 0 && (flags & O_TRUNCATE) && (flags & O_WRITE)) {
        result = truncate(&inode);
        if (result == 0) {
            inode.d.mtime = inode.d.ctime = (u32) rtc_seconds();
            result = write_inode(&inode);
        }
    }
    if (result != 0) {
        return result;
    }

    files[fd].open = 1;
    files[fd].flags = flags;
    files[fd].inode = inode.number;
    files[fd].pos = 0;
    files[fd].mount = fs;
    return fd;
}

ext2_fd ext2_open(const char *path, const int flags) {
    path = enter(path);
    const int refused = (flags & (O_WRITE | O_CREATE | O_TRUNCATE | O_APPEND)) ? unwritable() : unusable();
    if (refused != 0) {
        return refused;
    }
    return leave(open_file(path, flags));
}

static bool has_open_files(const struct mount *m) {
    for (int i = 0; i < MAX_FILES; i++) {
        if (files[i].open && files[i].mount == m) {
            return 1;
        }
    }
    return 0;
}

ext2_raw_fd ext2_open_raw(const char *path, const int flags, const bool force, const u32 serial) {
    const int result = enter_mount_point(path, serial);
    if (result != 0) {
        return result;
    }
    if (fs->detached) {
        return FS_BUSY;
    }
    int fd = 0;
    while (fd < MAX_RAW && raws[fd].open) {
        fd++;
    }
    if (fd == MAX_RAW) {
        return FS_TOO_MANY;
    }
    const int write = (flags & O_WRITE) != 0;
    if (write) {
        if (fs == root) {
            return FS_IS_ROOT;
        }
        if (!force && fs->medium != FS_MEDIUM_BLANK) {
            return FS_NOT_BLANK;
        }
        if (vblk_read_only(fs->dev)) {
            return FS_READ_ONLY;
        }
        if (has_open_files(fs) || fs->raw_readers > 0) {
            return FS_BUSY;
        }
        fs->detached = 1;
    } else {
        for (int i = 0; i < MAX_FILES; i++) {
            if (files[i].open && files[i].mount == fs && (files[i].flags & O_WRITE)) {
                return FS_BUSY;
            }
        }
        fs->raw_readers++;
    }
    raws[fd] = (struct raw) {.open = 1, .write = write, .mount = fs};
    return fd;
}

/* ---------------------------------------------------------------------------------------- */
/* Modifying */

int ext2_mkdir(const char *path) {
    path = enter(path);
    const int refused = unwritable();
    if (refused != 0) {
        return refused;
    }
    return leave(create(path, 1, NULL));
}

static bool is_open(const u32 inode) {
    for (int i = 0; i < MAX_FILES; i++) {
        if (files[i].open && files[i].mount == fs && files[i].inode == inode) {
            return 1;
        }
    }
    return 0;
}

static void unlink_inode(struct inode *inode) {
    inode->d.links_count--;
    if (inode->d.links_count == 0) {
        inode->d.dtime = (u32) rtc_seconds();
        if (truncate(inode) == 0) {
            free_inode(inode);
        }
    } else {
        inode->d.ctime = (u32) rtc_seconds();
        write_inode(inode);
    }
}

static int rename_path(const char *from, const char *to) {
    struct inode src_parent, dst_parent, inode;
    const char *src_name, *dst_name;
    u32 src_len, dst_len;
    struct location src_loc, dst_loc;
    int result = lookup(from, &src_parent, &src_name, &src_len);
    if (result == 0) {
        result = dir_find(&src_parent, src_name, src_len, &src_loc);
    }
    if (result == 0) {
        result = lookup(to, &dst_parent, &dst_name, &dst_len);
    }
    if (result == 0 && !is_dir(&dst_parent)) {
        result = FS_NOT_DIR;
    }
    if (result != 0) {
        return result;
    }
    if (!fs->writable) {
        return FS_READ_ONLY;
    }
    result = read_inode(src_loc.inode, &inode);
    if (result != 0) {
        return result;
    }
    const int type = is_dir(&inode) ? FT_DIR : FT_REG;

    struct inode replaced;
    const int found = dir_find(&dst_parent, dst_name, dst_len, &dst_loc);
    if (found == 0) {
        if (dst_loc.inode == inode.number) {
            return 0;
        }
        result = read_inode(dst_loc.inode, &replaced);
        if (result != 0) {
            return result;
        }
        if (is_dir(&replaced) || is_dir(&inode)) {
            return FS_EXISTS;
        }
        if (is_open(replaced.number)) {
            return FS_BUSY;
        }
    } else if (found != FS_NOT_FOUND) {
        return found;
    }

    const int moving_dir = is_dir(&inode) && src_parent.number != dst_parent.number;
    if (moving_dir) {
        struct inode walk = dst_parent;
        for (u32 depth = 0; walk.number != ROOT_INODE; depth++) {
            if (depth > SB->inodes_count) {
                return FS_IO;
            }
            if (walk.number == inode.number) {
                return FS_INVALID;
            }
            struct location up;
            if (dir_find(&walk, "..", 2, &up) != 0 || read_inode(up.inode, &walk) != 0) {
                return FS_IO;
            }
        }
    }

    struct slot slot;
    if (found != 0) {
        result = dir_reserve(&dst_parent, dst_len, &slot);
        if (result != 0) {
            return result;
        }
    }

    if (moving_dir) {
        dst_parent.d.links_count++;
        result = write_inode(&dst_parent);
    } else if (!is_dir(&inode)) {
        inode.d.links_count++;
        result = write_inode(&inode);
    }
    if (result != 0) {
        return result;
    }
    result = found == 0
        ? dir_set_entry(&dst_parent, &dst_loc, inode.number, type)
        : dir_put(&dst_parent, &slot, dst_name, dst_len, inode.number, type);
    if (result != 0) {
        return result;
    }
    if (src_parent.number == dst_parent.number) {
        src_parent = dst_parent;
    }
    result = dir_find(&src_parent, src_name, src_len, &src_loc);
    if (result == 0) {
        result = dir_remove(&src_parent, &src_loc);
    }
    if (result != 0) {
        return result;
    }
    if (moving_dir) {
        result = dir_set_parent(&inode, dst_parent.number);
        if (result == 0) {
            src_parent.d.links_count--;
            result = write_inode(&src_parent);
        }
    } else if (!is_dir(&inode)) {
        inode.d.links_count--;
        result = write_inode(&inode);
    }
    if (result == 0 && found == 0) {
        unlink_inode(&replaced);
    }
    return result;
}

int ext2_rename(const char *from, const char *to) {
    check_media();
    to = select_mount(to);
    struct mount *target = fs;
    from = select_mount(from);
    const int refused = unwritable();
    if (refused != 0) {
        return refused;
    }
    if (fs != target) {
        return FS_INVALID;
    }
    fs->wrote = 0;
    return leave(rename_path(from, to));
}

static int remove_path(const char *path) {
    struct inode parent;
    const char *name;
    u32 len;
    int result = lookup(path, &parent, &name, &len);
    if (result != 0) {
        return result;
    }
    struct location loc;
    result = dir_find(&parent, name, len, &loc);
    if (result != 0) {
        return result;
    }
    if (!fs->writable) {
        return FS_READ_ONLY;
    }
    struct inode inode;
    result = read_inode(loc.inode, &inode);
    if (result != 0) {
        return result;
    }
    if (is_open(inode.number)) {
        return FS_BUSY;
    }
    if (is_dir(&inode) && !is_dir_empty(&inode)) {
        return FS_NOT_EMPTY;
    }
    result = dir_remove(&parent, &loc);
    if (result != 0) {
        return result;
    }
    if (is_dir(&inode)) {
        parent.d.links_count--;
        write_inode(&parent);
        inode.d.links_count = 0;
        inode.d.dtime = (u32) rtc_seconds();
        if (truncate(&inode) == 0) {
            free_inode(&inode);
        }
    } else {
        unlink_inode(&inode);
    }
    return 0;
}

int ext2_remove(const char *path) {
    path = enter(path);
    const int refused = unwritable();
    if (refused != 0) {
        return refused;
    }
    return leave(remove_path(path));
}

static void replace(const int device) {
    char old_path[sizeof(mounts[0]->path)] = "";
    if (mounts[device] != NULL) {
        strcpy(old_path, mounts[device]->path);
        unmount(mounts[device]);
    }
    remount(device);
    report(old_path, device);
}

int ext2_format(const char *path, const char *label, const bool force, const u32 serial) {
    int result = enter_mount_point(path, serial);
    if (result != 0) {
        return result;
    }
    if (fs == root) {
        return FS_IS_ROOT;
    }
    char current[FS_LABEL_LENGTH + 1];
    if (label == NULL) {
        read_label(fs, current);
        label = is_valid_label(current, strlen(current)) ? current : "";
    }
    if (*label != 0 && (!is_valid_label(label, strlen(label)))) {
        return FS_INVALID;
    }
    if (*label != 0 && is_name_taken(label, fs)) {
        return FS_EXISTS;
    }
    if (!force && fs->medium != FS_MEDIUM_BLANK) {
        return FS_NOT_BLANK;
    }
    if (vblk_read_only(fs->dev) || (!fs->writable && !force)) {
        return FS_READ_ONLY;
    }
    if (has_open_files(fs) || fs->raw_readers > 0 || fs->detached) {
        return FS_BUSY;
    }
    const int device = fs->device;
    result = ext2_mkfs(fs->dev, label);
    replace(device);
    return result;
}

int ext2_set_label(const char *path, const char *label, const u32 serial) {
    int result = enter_mount_point(path, serial);
    if (result != 0) {
        return result;
    }
    const size_t len = strlen(label);
    if (len != 0 && !is_valid_label(label, len)) {
        return FS_INVALID;
    }
    if ((result = unwritable()) != 0) {
        return result;
    }
    if (!fs->writable) {
        return FS_READ_ONLY;
    }
    char old_path[sizeof(fs->path)];
    strcpy(old_path, fs->path);
    if (fs != root) {
        if (len != 0 && is_name_taken(label, fs)) {
            return FS_EXISTS;
        }
        if (has_open_files(fs)) {
            return FS_BUSY;
        }
    }
    memset(fs->super.raw + VOLUME_NAME_OFFSET, 0, FS_LABEL_LENGTH);
    memcpy(fs->super.raw + VOLUME_NAME_OFFSET, label, len);
    fs->super_dirty = 1;
    const int flushed = leave(0);
    if (flushed != 0 || fs == root) {
        return flushed;
    }
    name_mount(fs);
    if (strcmp(old_path, fs->path) != 0) {
        report(old_path, fs->device);
    }
    return 0;
}

/* ---------------------------------------------------------------------------------------- */
/* Open files */

static int enter_file(const int fd, struct file **out) {
    check_media();
    if (fd < 0 || fd >= MAX_FILES || !files[fd].open) {
        return FS_INVALID;
    }
    if (files[fd].mount == NULL) {
        return FS_CHANGED;
    }
    fs = files[fd].mount;
    fs->wrote = 0;
    *out = &files[fd];
    return 0;
}

long ext2_read(const ext2_fd fd, void *dst, size_t len) {
    struct file *file;
    const int entered = enter_file(fd, &file);
    if (entered != 0) {
        return entered;
    }
    if (!(file->flags & O_READ)) {
        return FS_INVALID;
    }
    struct inode inode;
    const int result = read_inode(file->inode, &inode);
    if (result != 0) {
        return result;
    }
    if (file->pos >= inode.d.size) {
        return 0;
    }
    if (len > inode.d.size - file->pos) {
        len = inode.d.size - file->pos;
    }
    u8 *out = dst;
    size_t done = 0;
    while (done < len) {
        const u32 offset = (u32) (file->pos % fs->block_size);
        size_t chunk = fs->block_size - offset;
        if (chunk > len - done) {
            chunk = len - done;
        }
        int error = 0;
        const u32 block = bmap(&inode, (u32) (file->pos / fs->block_size), 0, &error, NULL);
        if (block == 0) {
            if (error != 0) {
                return done > 0 ? (long) done : error;
            }
            memset(out + done, 0, chunk);
        } else {
            struct buf *b = bread(block);
            if (b == NULL) {
                return done > 0 ? (long) done : FS_IO;
            }
            memcpy(out + done, b->data + offset, chunk);
            brelse(b);
        }
        done += chunk;
        file->pos += chunk;
    }
    return (long) done;
}

static long write_file(struct file *file, const u8 *in, const size_t len) {
    struct inode inode;
    int result = read_inode(file->inode, &inode);
    if (result != 0) {
        return result;
    }
    if (file->flags & O_APPEND) {
        file->pos = inode.d.size;
    }
    if (file->pos + len > 0xFFFFFFFFull) {
        return FS_NO_SPACE;
    }
    size_t done = 0;
    while (done < len) {
        const u32 offset = (u32) (file->pos % fs->block_size);
        size_t chunk = fs->block_size - offset;
        if (chunk > len - done) {
            chunk = len - done;
        }
        int error = 0;
        int fresh = 0;
        const u32 block = bmap(&inode, (u32) (file->pos / fs->block_size), 1, &error, &fresh);
        struct buf *b = NULL;
        if (block != 0) {
            b = fresh || chunk == fs->block_size ? bnew(block) : bread(block);
        }
        if (b == NULL) {
            result = error != 0 ? error : FS_IO;
            break;
        }
        memcpy(b->data + offset, in + done, chunk);
        error = bwrite(b);
        brelse(b);
        if (error != 0) {
            result = error;
            break;
        }
        done += chunk;
        file->pos += chunk;
        if (file->pos > inode.d.size) {
            inode.d.size = (u32) file->pos;
        }
    }
    inode.d.mtime = inode.d.ctime = (u32) rtc_seconds();
    const int written = write_inode(&inode);
    if (done == 0) {
        return result != 0 ? result : written;
    }
    return (long) done;
}

long ext2_write(const ext2_fd fd, const void *src, const size_t len) {
    struct file *file;
    const int entered = enter_file(fd, &file);
    if (entered != 0) {
        return entered;
    }
    if (!(file->flags & O_WRITE)) {
        return FS_INVALID;
    }
    const long result = write_file(file, src, len);
    const int status = leave(result < 0 ? (int) result : 0);
    return result < 0 || status == 0 ? result : status;
}

long ext2_seek(const ext2_fd fd, const long offset, const int whence) {
    struct file *file;
    const int entered = enter_file(fd, &file);
    if (entered != 0) {
        return entered;
    }
    long base = 0;
    if (whence == 1) {
        base = (long) file->pos;
    } else if (whence == 2) {
        struct inode inode;
        const int result = read_inode(file->inode, &inode);
        if (result != 0) {
            return result;
        }
        base = inode.d.size;
    }
    if (base + offset < 0) {
        return FS_INVALID;
    }
    file->pos = (u64) (base + offset);
    return (long) file->pos;
}

int ext2_close(const ext2_fd fd) {
    if (fd < 0 || fd >= MAX_FILES || !files[fd].open) {
        return FS_INVALID;
    }
    files[fd].open = 0;
    return 0;
}

static long enter_raw(const int fd, const size_t len, struct raw **out) {
    if (fd < 0 || fd >= MAX_RAW || !raws[fd].open) {
        return FS_INVALID;
    }
    struct raw *raw = &raws[fd];
    if (raw->mount == NULL) {
        return FS_CHANGED;
    }
    if ((raw->pos | len) % SECTOR_SIZE != 0) {
        return FS_INVALID;
    }
    fs = raw->mount;
    const u64 capacity = vblk_capacity(fs->dev) * SECTOR_SIZE;
    *out = raw;
    return raw->pos >= capacity ? 0 : (long) (len < capacity - raw->pos ? len : capacity - raw->pos);
}

static long raw_transfer(const int write, const int fd, void *data, const size_t len) {
    struct raw *raw;
    const long count = enter_raw(fd, len, &raw);
    if (count <= 0) {
        return count;
    }
    for (long done = 0; done < count;) {
        const u32 chunk = count - done < RAW_CHUNK ? (u32) (count - done) : RAW_CHUNK;
        const int result = transfer(write, raw->pos / SECTOR_SIZE, (u8 *) data + done, chunk);
        if (result != 0) {
            return done > 0 ? done : result;
        }
        raw->pos += chunk;
        done += chunk;
    }
    return count;
}

long ext2_raw_read(const ext2_raw_fd fd, void *dst, const size_t len) {
    return raw_transfer(0, fd, dst, len);
}

long ext2_raw_write(const ext2_raw_fd fd, const void *src, const size_t len) {
    if (fd >= 0 && fd < MAX_RAW && raws[fd].open && !raws[fd].write) {
        return FS_INVALID;
    }
    const long count = raw_transfer(1, fd, (void *) src, len);
    return count == 0 && len > 0 ? FS_NO_SPACE : count;
}

long ext2_raw_seek(const ext2_raw_fd fd, const long offset, const int whence) {
    struct raw *raw;
    const long checked = enter_raw(fd, 0, &raw);
    if (checked < 0) {
        return checked;
    }
    const long capacity = (long) (vblk_capacity(fs->dev) * SECTOR_SIZE);
    const long base = whence == SEEK_SET ? 0 : whence == SEEK_CUR ? (long) raw->pos : capacity;
    const long pos = base + offset;
    if (pos < 0 || pos % SECTOR_SIZE != 0) {
        return FS_INVALID;
    }
    raw->pos = (u64) pos;
    return pos;
}

int ext2_raw_close(const ext2_raw_fd fd) {
    if (fd < 0 || fd >= MAX_RAW || !raws[fd].open) {
        return FS_INVALID;
    }
    struct raw *raw = &raws[fd];
    raw->open = 0;
    if (raw->mount == NULL) {
        return 0;
    }
    if (raw->write) {
        replace(raw->mount->device);
    } else {
        raw->mount->raw_readers--;
    }
    return 0;
}
