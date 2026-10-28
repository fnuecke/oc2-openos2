/* SPDX-License-Identifier: MIT */
/*
 * Writes a new, empty ext2 file system, like mke2fs -t ext2 -b 1024
 * -O ^ext_attr,^resize_inode,^dir_index,^large_file: revision 1, 1 KiB blocks, 256-byte inodes,
 * the filetype and sparse_super features, 5% reserved blocks, a root directory and lost+found.
 * The first 16 KiB are zeroed first and written last, each in one request, so an interrupted
 * format leaves a blank medium (see FS_MEDIUM_BLANK). A medium change during the writes ends it,
 * see write_block.
 */

#include <stdlib.h>
#include <string.h>
#include "drivers/rtc.h"
#include "drivers/virtio/virtio.h"
#include "ext2.h"
#include "internal.h"

#define BLOCK_SIZE 1024
#define BLOCKS_PER_GROUP (8 * BLOCK_SIZE)
#define INODE_SIZE 256
#define INODES_PER_BLOCK (BLOCK_SIZE / INODE_SIZE)
#define GROUP_DESC_SIZE 32
#define EXTRA_ISIZE 32
#define FIRST_INO 11
#define LOST_FOUND_INO 11
#define LOST_FOUND_BLOCKS 12
#define MIN_BLOCKS 60
#define MIB (1024 * 1024ull)
#define HEAD_BLOCKS (EXT2_HEAD_SIZE / BLOCK_SIZE)
#define FIRST_DATA_BLOCK (EXT2_SUPERBLOCK_OFFSET / BLOCK_SIZE)

#define S_INODES_COUNT 0x00
#define S_BLOCKS_COUNT 0x04
#define S_R_BLOCKS_COUNT 0x08
#define S_FREE_BLOCKS_COUNT 0x0C
#define S_FREE_INODES_COUNT 0x10
#define S_FIRST_DATA_BLOCK 0x14
#define S_BLOCKS_PER_GROUP 0x20
#define S_FRAGS_PER_GROUP 0x24
#define S_INODES_PER_GROUP 0x28
#define S_WTIME 0x30
#define S_MAX_MNT_COUNT 0x36
#define S_MAGIC 0x38
#define S_STATE 0x3A
#define S_ERRORS 0x3C
#define S_LASTCHECK 0x40
#define S_REV_LEVEL 0x4C
#define S_FIRST_INO 0x54
#define S_INODE_SIZE 0x58
#define S_BLOCK_GROUP_NR 0x5A
#define S_FEATURE_INCOMPAT 0x60
#define S_FEATURE_RO_COMPAT 0x64
#define S_UUID 0x68
#define S_HASH_SEED 0xEC
#define S_DEF_HASH_VERSION 0xFC
#define S_DEFAULT_MOUNT_OPTS 0x100
#define S_MKFS_TIME 0x108
#define S_MIN_EXTRA_ISIZE 0x15C
#define S_WANT_EXTRA_ISIZE 0x15E
#define S_FLAGS 0x160

#define G_BLOCK_BITMAP 0x00
#define G_INODE_BITMAP 0x04
#define G_INODE_TABLE 0x08
#define G_FREE_BLOCKS_COUNT 0x0C
#define G_FREE_INODES_COUNT 0x0E
#define G_USED_DIRS_COUNT 0x10
#define G_FLAGS 0x12
#define BG_INODE_ZEROED 0x4

#define I_MODE 0x00
#define I_SIZE 0x04
#define I_ATIME 0x08
#define I_CTIME 0x0C
#define I_MTIME 0x10
#define I_LINKS_COUNT 0x1A
#define I_BLOCKS 0x1C
#define I_BLOCK 0x28
#define I_EXTRA_ISIZE 0x80
#define I_CRTIME 0x90

#define ERRORS_CONTINUE 1
#define HASH_HALF_MD4 1
#define DEFAULT_MOUNT_XATTR_USER 0x4
#define DEFAULT_MOUNT_ACL 0x8
#define FLAGS_SIGNED_HASH 0x1

struct pass {
    struct vblk *dev;
    u8 *head; /* NULL in the first pass */
};

struct layout {
    u32 blocks;
    u32 groups;
    u32 inodes_per_group;
    u32 table_blocks;
    u32 gdt_blocks;
};

static void put16(u8 *p, const u32 offset, const u32 value) {
    p[offset] = (u8) value;
    p[offset + 1] = (u8) (value >> 8);
}

static void put32(u8 *p, const u32 offset, const u32 value) {
    put16(p, offset, value);
    put16(p, offset + 2, value >> 16);
}

static u32 group_start(const u32 group) {
    return FIRST_DATA_BLOCK + group * BLOCKS_PER_GROUP;
}

static u32 group_blocks(const struct layout *l, const u32 group) {
    const u32 left = l->blocks - group_start(group);
    return left < BLOCKS_PER_GROUP ? left : BLOCKS_PER_GROUP;
}

static bool is_power_of(u32 n, const u32 base) {
    while (n % base == 0) {
        n /= base;
    }
    return n == 1;
}

static bool has_super(const u32 group) {
    return group <= 1 || is_power_of(group, 3) || is_power_of(group, 5) || is_power_of(group, 7);
}

static u32 overhead(const struct layout *l, const u32 group) {
    return (has_super(group) ? 1 + l->gdt_blocks : 0) + 2 + l->table_blocks;
}

static int plan(const u64 sectors, struct layout *l) {
    const u64 bytes = sectors * VBLK_SECTOR_SIZE;
    l->blocks = bytes / BLOCK_SIZE > 0xFFFFFFFFull ? 0xFFFFFFFFu : (u32) (bytes / BLOCK_SIZE);
    l->blocks &= ~3u;
    const u32 ratio = bytes < 3 * MIB ? 8192 : bytes < 512 * MIB ? 4096 : 16384;
    for (;;) {
        if (l->blocks < MIN_BLOCKS) {
            return FS_NO_SPACE;
        }
        l->groups = (l->blocks - FIRST_DATA_BLOCK + BLOCKS_PER_GROUP - 1) / BLOCKS_PER_GROUP;
        const u64 inodes = (u64) l->blocks * BLOCK_SIZE / ratio;
        u64 per_group = (inodes + l->groups - 1) / l->groups;
        per_group = (per_group + INODES_PER_BLOCK - 1) / INODES_PER_BLOCK * INODES_PER_BLOCK & ~7ull;
        if (per_group < 16) {
            per_group = 16;
        }
        if (per_group > 8 * BLOCK_SIZE) {
            per_group = 8 * BLOCK_SIZE;
        }
        l->inodes_per_group = (u32) per_group;
        l->table_blocks = l->inodes_per_group / INODES_PER_BLOCK;
        l->gdt_blocks = (l->groups * GROUP_DESC_SIZE + BLOCK_SIZE - 1) / BLOCK_SIZE;
        const u32 last = l->groups - 1;
        if (l->groups > 1 && group_blocks(l, last) < overhead(l, last) + 50) {
            l->blocks -= group_blocks(l, last);
            continue;
        }
        if (overhead(l, 0) + 1 + LOST_FOUND_BLOCKS >= group_blocks(l, 0)) {
            return FS_NO_SPACE;
        }
        return 0;
    }
}

static int write_sectors(struct vblk *dev, const u64 sector, const void *data, const u32 len) {
    const int result = vblk_write(dev, sector, data, len);
    return result == 0 ? 0 : result == VBLK_CHANGED ? FS_CHANGED : FS_IO;
}

static int write_block(const struct pass *pass, const u32 block, const void *data) {
    if (block < HEAD_BLOCKS && pass->head != NULL) {
        memcpy(pass->head + (u64) block * BLOCK_SIZE, data, BLOCK_SIZE);
    } else if (block >= HEAD_BLOCKS && pass->head == NULL) {
        return write_sectors(pass->dev, (u64) block * (BLOCK_SIZE / VBLK_SECTOR_SIZE), data, BLOCK_SIZE);
    }
    return 0;
}

static void fill_bitmap(u8 *bitmap, const u32 used, const u32 valid) {
    memset(bitmap, 0, BLOCK_SIZE);
    for (u32 bit = 0; bit < 8 * BLOCK_SIZE; bit++) {
        if (bit < used || bit >= valid) {
            bitmap[bit / 8] |= (u8) (1u << (bit % 8));
        }
    }
}

static void put_dir_inode(u8 *inode, const u32 mode, const u32 links, const u32 first, const u32 count, const u32 now) {
    put16(inode, I_MODE, mode);
    put32(inode, I_SIZE, count * BLOCK_SIZE);
    put32(inode, I_ATIME, now);
    put32(inode, I_CTIME, now);
    put32(inode, I_MTIME, now);
    put16(inode, I_LINKS_COUNT, links);
    put32(inode, I_BLOCKS, count * (BLOCK_SIZE / EXT2_I_BLOCK_SIZE));
    for (u32 i = 0; i < count; i++) {
        put32(inode, I_BLOCK + 4 * i, first + i);
    }
    put16(inode, I_EXTRA_ISIZE, EXTRA_ISIZE);
    put32(inode, I_CRTIME, now);
}

static u32 put_entry(u8 *block, const u32 offset, const u32 inode, const char *name, const int last) {
    const u32 len = (u32) strlen(name);
    const u32 rec_len = last ? BLOCK_SIZE - offset : (8 + len + 3) / 4 * 4;
    put32(block, offset, inode);
    put16(block, offset + 4, rec_len);
    block[offset + 6] = (u8) len;
    block[offset + 7] = FT_DIR;
    memcpy(block + offset + 8, name, len);
    return offset + rec_len;
}

static void put_super(u8 *sb, const struct layout *l, const char *label, const u32 now) {
    u32 free_blocks = 0;
    for (u32 group = 0; group < l->groups; group++) {
        free_blocks += group_blocks(l, group) - overhead(l, group);
    }
    free_blocks -= 1 + LOST_FOUND_BLOCKS;
    memset(sb, 0, BLOCK_SIZE);
    put32(sb, S_INODES_COUNT, l->groups * l->inodes_per_group);
    put32(sb, S_BLOCKS_COUNT, l->blocks);
    put32(sb, S_R_BLOCKS_COUNT, l->blocks / 20);
    put32(sb, S_FREE_BLOCKS_COUNT, free_blocks);
    put32(sb, S_FREE_INODES_COUNT, l->groups * l->inodes_per_group - FIRST_INO);
    put32(sb, S_FIRST_DATA_BLOCK, FIRST_DATA_BLOCK);
    put32(sb, S_BLOCKS_PER_GROUP, BLOCKS_PER_GROUP);
    put32(sb, S_FRAGS_PER_GROUP, BLOCKS_PER_GROUP);
    put32(sb, S_INODES_PER_GROUP, l->inodes_per_group);
    put32(sb, S_WTIME, now);
    put16(sb, S_MAX_MNT_COUNT, 0xFFFF);
    put16(sb, S_MAGIC, EXT2_MAGIC);
    put16(sb, S_STATE, 1);
    put16(sb, S_ERRORS, ERRORS_CONTINUE);
    put32(sb, S_LASTCHECK, now);
    put32(sb, S_REV_LEVEL, 1);
    put32(sb, S_FIRST_INO, FIRST_INO);
    put16(sb, S_INODE_SIZE, INODE_SIZE);
    put32(sb, S_FEATURE_INCOMPAT, INCOMPAT_FILETYPE);
    put32(sb, S_FEATURE_RO_COMPAT, RO_COMPAT_SPARSE_SUPER);
    for (int i = 0; i < 4; i++) {
        put32(sb, S_UUID + 4 * i, (u32) mix64(entropy() + (u64) i));
        put32(sb, S_HASH_SEED + 4 * i, (u32) mix64(entropy() + (u64) i + 4));
    }
    sb[S_UUID + 6] = (u8) ((sb[S_UUID + 6] & 0x0F) | 0x40);
    sb[S_UUID + 8] = (u8) ((sb[S_UUID + 8] & 0x3F) | 0x80);
    strncpy((char *) sb + VOLUME_NAME_OFFSET, label, FS_LABEL_LENGTH);
    sb[S_DEF_HASH_VERSION] = HASH_HALF_MD4;
    put32(sb, S_DEFAULT_MOUNT_OPTS, DEFAULT_MOUNT_XATTR_USER | DEFAULT_MOUNT_ACL);
    put32(sb, S_MKFS_TIME, now);
    put16(sb, S_MIN_EXTRA_ISIZE, EXTRA_ISIZE);
    put16(sb, S_WANT_EXTRA_ISIZE, EXTRA_ISIZE);
    put32(sb, S_FLAGS, FLAGS_SIGNED_HASH);
}

static int write_groups(const struct pass *dev, const struct layout *l, u8 *gdt, u8 *buf, const u32 now) {
    const u32 root_block = group_start(0) + overhead(l, 0);
    for (u32 group = 0; group < l->groups; group++) {
        const u32 block_bitmap = group_start(group) + (has_super(group) ? 1 + l->gdt_blocks : 0);
        const u32 inode_bitmap = block_bitmap + 1;
        const u32 inode_table = inode_bitmap + 1;
        const u32 used_blocks = overhead(l, group) + (group == 0 ? 1 + LOST_FOUND_BLOCKS : 0);
        const u32 used_inodes = group == 0 ? FIRST_INO : 0;
        u8 *gd = gdt + group * GROUP_DESC_SIZE;
        put32(gd, G_BLOCK_BITMAP, block_bitmap);
        put32(gd, G_INODE_BITMAP, inode_bitmap);
        put32(gd, G_INODE_TABLE, inode_table);
        put16(gd, G_FREE_BLOCKS_COUNT, group_blocks(l, group) - used_blocks);
        put16(gd, G_FREE_INODES_COUNT, l->inodes_per_group - used_inodes);
        put16(gd, G_USED_DIRS_COUNT, group == 0 ? 2 : 0);
        put16(gd, G_FLAGS, BG_INODE_ZEROED);

        int result;
        fill_bitmap(buf, used_blocks, group_blocks(l, group));
        if ((result = write_block(dev, block_bitmap, buf)) != 0) {
            return result;
        }
        fill_bitmap(buf, used_inodes, l->inodes_per_group);
        if ((result = write_block(dev, inode_bitmap, buf)) != 0) {
            return result;
        }
        for (u32 i = 0; i < l->table_blocks; i++) {
            memset(buf, 0, BLOCK_SIZE);
            if (group == 0 && i == 0) {
                put32(buf, I_ATIME, now);
                put32(buf, I_CTIME, now);
                put32(buf, I_MTIME, now);
                put_dir_inode(buf + (ROOT_INODE - 1) * INODE_SIZE, EXT2_S_IFDIR | 0755, 3, root_block, 1, now);
            }
            if (group == 0 && i == (LOST_FOUND_INO - 1) / INODES_PER_BLOCK) {
                put_dir_inode(buf + (LOST_FOUND_INO - 1) % INODES_PER_BLOCK * INODE_SIZE, EXT2_S_IFDIR | 0700, 2,
                    root_block + 1, LOST_FOUND_BLOCKS, now);
            }
            if ((result = write_block(dev, inode_table + i, buf)) != 0) {
                return result;
            }
        }
    }
    return 0;
}

static int write_directories(const struct pass *dev, const struct layout *l, u8 *buf) {
    const u32 root_block = group_start(0) + overhead(l, 0);
    memset(buf, 0, BLOCK_SIZE);
    u32 offset = put_entry(buf, 0, ROOT_INODE, ".", 0);
    offset = put_entry(buf, offset, ROOT_INODE, "..", 0);
    put_entry(buf, offset, LOST_FOUND_INO, "lost+found", 1);
    int result = write_block(dev, root_block, buf);
    for (u32 i = 0; result == 0 && i < LOST_FOUND_BLOCKS; i++) {
        memset(buf, 0, BLOCK_SIZE);
        if (i == 0) {
            put_entry(buf, put_entry(buf, 0, LOST_FOUND_INO, ".", 0), ROOT_INODE, "..", 1);
        } else {
            put16(buf, 4, BLOCK_SIZE);
        }
        result = write_block(dev, root_block + 1 + i, buf);
    }
    return result;
}

static int write_metadata(const struct pass *dev, const struct layout *l, const u8 *gdt, u8 *sb) {
    for (u32 n = l->groups; n > 0; n--) {
        const u32 group = n - 1;
        if (!has_super(group)) {
            continue;
        }
        for (u32 i = 0; i < l->gdt_blocks; i++) {
            const int result = write_block(dev, group_start(group) + 1 + i, gdt + (u64) i * BLOCK_SIZE);
            if (result != 0) {
                return result;
            }
        }
        put16(sb, S_BLOCK_GROUP_NR, group);
        put16(sb, S_STATE, group == 0 ? 1 : 0);
        const int result = write_block(dev, group_start(group), sb);
        if (result != 0) {
            return result;
        }
    }
    return 0;
}

int ext2_mkfs(struct vblk *dev, const char *label) {
    if (vblk_read_only(dev)) {
        return FS_READ_ONLY;
    }
    struct layout l;
    int result = plan(vblk_capacity(dev), &l);
    if (result != 0) {
        return result;
    }
    u8 *buf = calloc(1, BLOCK_SIZE);
    u8 *sb = malloc(BLOCK_SIZE);
    u8 *gdt = calloc(l.gdt_blocks, BLOCK_SIZE);
    u8 *head = calloc(HEAD_BLOCKS, BLOCK_SIZE);
    if (buf == NULL || sb == NULL || gdt == NULL || head == NULL) {
        free(buf);
        free(sb);
        free(gdt);
        free(head);
        return FS_NO_SPACE;
    }
    const u32 now = (u32) rtc_seconds();
    put_super(sb, &l, label, now);
    const u32 head_size = (l.blocks < HEAD_BLOCKS ? l.blocks : HEAD_BLOCKS) * BLOCK_SIZE;
    result = write_sectors(dev, 0, head, head_size);
    for (int last = 0; result == 0 && last <= 1; last++) {
        const struct pass pass = {dev, last ? head : NULL};
        memset(gdt, 0, (size_t) l.gdt_blocks * BLOCK_SIZE);
        result = write_groups(&pass, &l, gdt, buf, now);
        if (result == 0) {
            result = write_directories(&pass, &l, buf);
        }
        if (result == 0) {
            result = write_metadata(&pass, &l, gdt, sb);
        }
    }
    if (result == 0) {
        result = write_sectors(dev, 0, head, head_size);
    }
    free(buf);
    free(sb);
    free(head);
    free(gdt);
    return result;
}
