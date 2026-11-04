// SPDX-License-Identifier: MIT

use super::{DRIVERS, Driver};
use crate::api::kprintf;
use crate::arch::{KernelCell, Mmio};
use crate::fdt::Node;
use crate::fs::devfs::{self, DEVICES};
use crate::fs::sysfs::{self, CLASSES, Content};
use crate::fs::{Error, Name, Result};

register!(DRIVERS, &Driver { compatible: &[b"simple-framebuffer"], probe });
register!(DEVICES, &Framebuffers);
register!(CLASSES, &Framebuffers);

struct Framebuffers;

impl devfs::Device for Framebuffers {
    fn count(&self) -> usize {
        count()
    }

    fn name(&self, index: usize) -> Option<Name> {
        name(index)
    }

    fn read(&self, index: usize, pos: u64, dst: &mut [u8]) -> Result<usize> {
        let fb = framebuffer(index).ok_or(Error::NotFound)?;
        let size = fb.size();
        if pos >= size {
            return Ok(0);
        }
        let count = (size - pos).min(dst.len() as u64) as usize;
        load(fb.mmio, pos, &mut dst[..count]);
        Ok(count)
    }

    fn write(&self, index: usize, pos: u64, src: &[u8]) -> Result<usize> {
        let fb = framebuffer(index).ok_or(Error::NotFound)?;
        let size = fb.size();
        if pos > size {
            return Err(Error::FileTooBig);
        }
        let count = (size - pos).min(src.len() as u64) as usize;
        if count == 0 && !src.is_empty() {
            return Err(Error::NoSpace);
        }
        store(fb.mmio, pos, &src[..count]);
        Ok(count)
    }

    fn seekable(&self) -> bool {
        true
    }

    fn size(&self, index: usize) -> Result<u64> {
        framebuffer(index).map(Framebuffer::size).ok_or(Error::NotFound)
    }
}

impl sysfs::Class for Framebuffers {
    fn name(&self) -> &'static [u8] {
        b"graphics"
    }

    fn count(&self) -> usize {
        count()
    }

    fn instance(&self, index: usize) -> Option<Name> {
        name(index)
    }

    fn attributes(&self) -> &'static [&'static [u8]] {
        &ATTRIBUTES
    }

    fn show(&self, index: usize, attribute: usize, out: &mut Content) -> Option<()> {
        let fb = framebuffer(index)?;
        match attribute {
            BITS_PER_PIXEL => write!(out, "{R5G6B5_BITS}"),
            FORMAT => write!(out, "{R5G6B5}"),
            NAME => write!(out, "simple"),
            STRIDE => write!(out, "{}", fb.stride),
            VIRTUAL_SIZE => write!(out, "{},{}", fb.width, fb.height),
            _ => return None,
        }
        Some(())
    }
}

const BITS_PER_PIXEL: usize = 0;
const FORMAT: usize = 1;
const NAME: usize = 2;
const STRIDE: usize = 3;
const VIRTUAL_SIZE: usize = 4;
const ATTRIBUTES: [&[u8]; 5] = [b"bits_per_pixel", b"format", b"name", b"stride", b"virtual_size"];
const R5G6B5: &str = "r5g6b5";
const R5G6B5_BITS: u32 = 16;
const WORD: usize = size_of::<u64>();
const MAX_FRAMEBUFFERS: usize = 4;

static FRAMEBUFFERS: KernelCell<[Option<Framebuffer>; MAX_FRAMEBUFFERS]> = KernelCell::new([None; MAX_FRAMEBUFFERS]);

fn probe(node: &Node) {
    let Some((base, length)) = node.reg() else { return };
    let (Some(width), Some(height), Some(stride)) = (node.u32(b"width"), node.u32(b"height"), node.u32(b"stride"))
    else {
        return;
    };
    let fb = Framebuffer { mmio: unsafe { Mmio::new(base) }, width, height, stride };
    let format = node.prop(b"format").and_then(|f| f.strip_suffix(b"\0"));
    if format != Some(R5G6B5.as_bytes()) || fb.size() > length {
        unsafe { kprintf(c"system: ignoring framebuffer at %lx\n".as_ptr(), base) };
        return;
    }
    FRAMEBUFFERS.with(|fbs| {
        if let Some(free) = fbs.iter_mut().find(|fb| fb.is_none()) {
            *free = Some(fb);
        }
    });
}

fn count() -> usize {
    FRAMEBUFFERS.with(|fbs| fbs.iter().take_while(|fb| fb.is_some()).count())
}

fn framebuffer(index: usize) -> Option<Framebuffer> {
    FRAMEBUFFERS.with(|fbs| fbs.get(index).copied().flatten())
}

fn name(index: usize) -> Option<Name> {
    framebuffer(index)?;
    let mut name = Name::EMPTY;
    write!(name, "fb{index}");
    Some(name)
}

fn head(address: u64, len: usize) -> usize {
    ((address.wrapping_neg() % WORD as u64) as usize).min(len)
}

fn narrow(address: u64, len: usize) -> usize {
    [4, 2].into_iter().find(|&size| address.is_multiple_of(size as u64) && len >= size).unwrap_or(1)
}

fn load(mmio: Mmio, offset: u64, dst: &mut [u8]) {
    let (head, rest) = dst.split_at_mut(head(mmio.base() + offset, dst.len()));
    load_narrow(mmio, offset, head);
    let mut at = offset + head.len() as u64;
    let tail = match unsafe { rest.align_to_mut::<u64>() } {
        ([], words, tail) => {
            for word in words {
                *word = u64::from_le(mmio.read64(at));
                at += WORD as u64;
            }
            tail
        }
        _ => {
            let (words, tail) = rest.as_chunks_mut::<WORD>();
            for word in words {
                *word = mmio.read64(at).to_le_bytes();
                at += WORD as u64;
            }
            tail
        }
    };
    load_narrow(mmio, at, tail);
}

fn load_narrow(mmio: Mmio, mut at: u64, mut dst: &mut [u8]) {
    while !dst.is_empty() {
        let size = narrow(mmio.base() + at, dst.len());
        let (chunk, rest) = core::mem::take(&mut dst).split_at_mut(size);
        match chunk {
            [a, b, c, d] => [*a, *b, *c, *d] = mmio.read32(at).to_le_bytes(),
            [a, b] => [*a, *b] = mmio.read16(at).to_le_bytes(),
            [a] => *a = mmio.read8(at),
            _ => {}
        }
        at += size as u64;
        dst = rest;
    }
}

fn store(mmio: Mmio, offset: u64, src: &[u8]) {
    let (head, rest) = src.split_at(head(mmio.base() + offset, src.len()));
    store_narrow(mmio, offset, head);
    let mut at = offset + head.len() as u64;
    let tail = match unsafe { rest.align_to::<u64>() } {
        ([], words, tail) => {
            for &word in words {
                mmio.write64(at, u64::from_le(word));
                at += WORD as u64;
            }
            tail
        }
        _ => {
            let (words, tail) = rest.as_chunks::<WORD>();
            for word in words {
                mmio.write64(at, u64::from_le_bytes(*word));
                at += WORD as u64;
            }
            tail
        }
    };
    store_narrow(mmio, at, tail);
}

fn store_narrow(mmio: Mmio, mut at: u64, mut src: &[u8]) {
    while !src.is_empty() {
        let size = narrow(mmio.base() + at, src.len());
        let (chunk, rest) = src.split_at(size);
        match *chunk {
            [a, b, c, d] => mmio.write32(at, u32::from_le_bytes([a, b, c, d])),
            [a, b] => mmio.write16(at, u16::from_le_bytes([a, b])),
            [a] => mmio.write8(at, a),
            _ => {}
        }
        at += size as u64;
        src = rest;
    }
}

#[derive(Clone, Copy)]
struct Framebuffer {
    mmio: Mmio,
    width: u32,
    height: u32,
    stride: u32, // bytes per line
}

impl Framebuffer {
    fn size(self) -> u64 {
        self.stride as u64 * self.height as u64
    }
}
