#!/bin/sh
# SPDX-License-Identifier: MIT
# Builds an ext2 image with the kernel at /boot/Image and the contents of the given directories at
# its root, later directories replacing files of earlier ones.
# Usage: mkdisk.sh <out.img> <Image> <dir>...
# SIZE (default 2M), BLOCK_SIZE (default 1024) and LABEL (default openos2) may be set in the
# environment.
#
# The image only depends on its inputs: the UUID and directory hash seed are fixed, all timestamps
# are SOURCE_DATE_EPOCH (default 2026-01-01) and all files belong to root.
set -eu

out=$1
image=$2
shift 2

uuid=6f63322d-6f73-4000-8000-000000000001
epoch=${SOURCE_DATE_EPOCH:-1767225600}

staging=$(mktemp -d)
trap 'rm -rf "$staging"' EXIT

for dir in "$@"; do
    cp -R --preserve=links "$dir"/. "$staging"/
done
mkdir -p "$staging/boot" "$staging/mnt"
cp "$image" "$staging/boot/Image"
find "$staging" -exec touch -h -d "@$epoch" {} +

rm -f "$out"
export E2FSPROGS_FAKE_TIME="$epoch"
mke2fs -q -F -t ext2 -b "${BLOCK_SIZE:-1024}" -U "$uuid" -L "${LABEL-openos2}" \
    -i 4096 -E root_owner=0:0,hash_seed="$uuid" -d "$staging" "$out" "${SIZE:-2M}"

commands=$(mktemp)
(cd "$staging" && find . -mindepth 1) | while read -r path; do
    path=${path#.}
    for field in uid gid; do
        echo "set_inode_field $path $field 0"
    done
    for field in ctime crtime; do
        echo "set_inode_field $path $field @$epoch"
    done
done > "$commands"
debugfs -w -f "$commands" "$out" >/dev/null 2>&1
rm -f "$commands"
