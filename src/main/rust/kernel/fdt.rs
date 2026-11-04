// SPDX-License-Identifier: MIT

use crate::bytes::{be32, until_nul};

const MAGIC: u32 = 0xD00DFEED;
const BEGIN_NODE: u32 = 1;
const END_NODE: u32 = 2;
const PROP: u32 = 3;
const NOP: u32 = 4;
const MAX_DEPTH: usize = 8;

#[derive(Clone, Copy)]
pub struct Fdt {
    dtb: *const u8,
    size: usize,
}

#[derive(Clone, Copy)]
pub struct Node<'a> {
    pub name: &'a [u8], // includes unit address, e.g. "uart@10000000".
    pub parent: &'a [u8],
    structure: &'a [u8],
    props: usize, // the offset of the first token after the name
    strings: &'a [u8],
    address_cells: usize, // the parent's, for reg
    size_cells: usize,
}

impl Fdt {
    pub const EMPTY: Self = Self { dtb: core::ptr::null(), size: 0 };

    pub unsafe fn new(dtb: *const u8) -> Option<Self> {
        let header = unsafe { core::slice::from_raw_parts(dtb, 8) };
        if be32(header, 0) != Some(MAGIC) {
            return None;
        }
        Some(Self { dtb, size: be32(header, 4)? as usize })
    }

    pub fn as_bytes(self) -> &'static [u8] {
        if self.dtb.is_null() { &[] } else { unsafe { core::slice::from_raw_parts(self.dtb, self.size) } }
    }

    pub fn root(self) -> Option<Node<'static>> {
        let (structure, strings) = self.blocks()?;
        if be32(structure, 0)? != BEGIN_NODE {
            return None;
        }
        let name = until_nul(structure.get(4..)?);
        let props = 4 + ((name.len() + 4) & !3);
        Some(Node { name, parent: &[], structure, props, strings, address_cells: 2, size_cells: 1 })
    }

    pub fn walk(self, f: &mut dyn FnMut(&Node<'static>)) {
        let Some((structure, strings)) = self.blocks() else { return };

        let mut stack = [(&[][..], 2, 1); MAX_DEPTH + 1];
        let mut depth = 0;
        let mut skipped = 0;
        let mut offset = 0;
        loop {
            let Some(token) = be32(structure, offset) else { return };
            offset += 4;
            match token {
                BEGIN_NODE => {
                    let name = until_nul(&structure[offset..]);
                    offset += (name.len() + 4) & !3;
                    if depth == MAX_DEPTH || skipped > 0 {
                        skipped += 1;
                        continue;
                    }
                    let (parent, address_cells, size_cells) = stack[depth];
                    let node = Node { name, parent, structure, props: offset, strings, address_cells, size_cells };
                    let cells = |name, default| node.u32(name).map_or(default, |n| n as usize);
                    depth += 1;
                    stack[depth] = (name, cells(b"#address-cells", 2), cells(b"#size-cells", 1));
                    f(&node);
                }
                END_NODE if skipped > 0 => skipped -= 1,
                END_NODE if depth > 0 => depth -= 1,
                PROP => {
                    let Some(length) = be32(structure, offset) else { return };
                    offset += 8 + ((length as usize + 3) & !3);
                }
                NOP => {}
                _ => return,
            }
        }
    }

    pub fn with_phandle(self, phandle: u32) -> Option<Node<'static>> {
        let mut found = None;
        self.walk(&mut |node| {
            if found.is_none() && node.u32(b"phandle") == Some(phandle) {
                found = Some(*node);
            }
        });
        found
    }

    fn blocks(self) -> Option<(&'static [u8], &'static [u8])> {
        let dt = self.as_bytes();
        let block = |at| be32(dt, at).and_then(|offset| dt.get(offset as usize..));
        Some((block(8)?, block(12)?))
    }
}

impl<'a> Node<'a> {
    pub fn prop(&self, name: &[u8]) -> Option<&'a [u8]> {
        self.props().find(|&(n, _)| n == name).map(|(_, value)| value)
    }

    pub fn props(&self) -> impl Iterator<Item = (&'a [u8], &'a [u8])> + use<'a> {
        let (structure, strings) = (self.structure, self.strings);
        let mut offset = self.props;
        core::iter::from_fn(move || {
            loop {
                match be32(structure, offset)? {
                    PROP => {
                        let length = be32(structure, offset + 4)? as usize;
                        let name = until_nul(strings.get(be32(structure, offset + 8)? as usize..)?);
                        let at = offset + 12;
                        offset = at + ((length + 3) & !3);
                        return Some((name, structure.get(at..at + length)?));
                    }
                    NOP => offset += 4,
                    _ => return None,
                }
            }
        })
    }

    pub fn children(&self) -> impl Iterator<Item = Node<'a>> + use<'a> {
        let cells = |name, default| self.u32(name).map_or(default, |n| n as usize);
        let (address_cells, size_cells) = (cells(b"#address-cells", 2), cells(b"#size-cells", 1));
        let (parent, structure, strings) = (self.name, self.structure, self.strings);
        let mut offset = self.props;
        let mut depth = 0;
        core::iter::from_fn(move || {
            loop {
                let token = be32(structure, offset)?;
                offset += 4;
                match token {
                    BEGIN_NODE => {
                        let name = until_nul(structure.get(offset..)?);
                        offset += (name.len() + 4) & !3;
                        depth += 1;
                        if depth == 1 {
                            return Some(Node {
                                name,
                                parent,
                                structure,
                                props: offset,
                                strings,
                                address_cells,
                                size_cells,
                            });
                        }
                    }
                    END_NODE if depth > 0 => depth -= 1,
                    PROP => offset += 8 + ((be32(structure, offset)? as usize + 3) & !3),
                    NOP => {}
                    _ => return None,
                }
            }
        })
    }

    pub fn u32(&self, name: &[u8]) -> Option<u32> {
        self.prop(name).filter(|value| value.len() == 4).and_then(|value| be32(value, 0))
    }

    pub fn is_compatible(&self, any: &[&[u8]]) -> bool {
        self.prop(b"compatible").is_some_and(|list| list.split(|&c| c == 0).any(|s| any.contains(&s)))
    }

    pub fn reg(&self) -> Option<(u64, u64)> {
        let value = self.prop(b"reg")?;
        let cells =
            |at: usize, count: usize| (at..at + count).try_fold(0, |v, i| Some(v << 32 | be32(value, 4 * i)? as u64));
        Some((cells(0, self.address_cells)?, cells(self.address_cells, self.size_cells)?))
    }

    pub fn irq(&self) -> Option<u32> {
        match (self.prop(b"interrupts"), self.prop(b"interrupts-extended")) {
            (Some(value), _) if value.len() >= 4 => be32(value, 0),
            (_, Some(value)) if value.len() >= 8 => be32(value, 4),
            _ => None,
        }
    }
}
