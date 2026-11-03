// SPDX-License-Identifier: MIT

pub fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(*b.get(at..)?.first_chunk()?))
}

pub fn le16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(*b.get(at..)?.first_chunk()?))
}

pub fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(*b.get(at..)?.first_chunk()?))
}

pub fn le64(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(*b.get(at..)?.first_chunk()?))
}

pub fn until_nul(b: &[u8]) -> &[u8] {
    &b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]
}
