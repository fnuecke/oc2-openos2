// SPDX-License-Identifier: MIT

use crate::arch::{IRQ_S_EXTERNAL, RawCell};
use crate::bytes::{be32, until_nul};
use crate::fdt::Fdt;

#[derive(Clone, Copy, Default)]
pub struct Region {
    pub base: u64,
    pub size: u64,
}

pub struct Board {
    pub fdt: Fdt,
    pub ram: [Region; MAX_RAM],
    pub ram_count: usize,
    pub ram_total: u64,
    pub reserved: Region,
    pub timebase: u64,
    pub plic: u64,
    pub plic_context: u64, // the S-mode context of the hart
    pub stdout: Option<u64>,
    pub root: usize, // the block device of root=/dev/vdX in /chosen/bootargs, else 0
}

const MAX_RAM: usize = 16;
const RAM_VIRTUAL_OFFSET: u64 = 0xFFFF_FFFF_0000_0000;

static BOARD: RawCell<Board> = RawCell::new(Board {
    fdt: Fdt::EMPTY,
    ram: [Region { base: 0, size: 0 }; MAX_RAM],
    ram_count: 0,
    ram_total: 0,
    reserved: Region { base: 0, size: 0 },
    timebase: 0,
    plic: 0,
    plic_context: 0,
    stdout: None,
    root: 0,
});

pub fn board() -> &'static Board {
    unsafe { &*BOARD.get() }
}

pub unsafe fn scan(dtb: *const u8) -> Result<(), ()> {
    let fdt = unsafe { Fdt::new(dtb) }.ok_or(())?;
    let mut board = Board {
        fdt,
        ram: [Region::default(); MAX_RAM],
        ram_count: 0,
        ram_total: 0,
        reserved: Region::default(),
        timebase: 0,
        plic: 0,
        plic_context: 0,
        stdout: None,
        root: 0,
    };
    let mut plic = None;
    fdt.walk(&mut |node| {
        if let Some(timebase) = node.u32(b"timebase-frequency") {
            board.timebase = timebase as u64;
        }
        let reg = node.reg().map(|(base, size)| Region { base, size });
        if node.prop(b"device_type") == Some(b"memory\0")
            && let Some(region) = reg
        {
            board.ram_total += region.size;
            if board.ram_count < MAX_RAM {
                board.ram[board.ram_count] = region;
                board.ram_count += 1;
            }
        } else if node.parent == b"reserved-memory"
            && let Some(region) = reg.filter(|_| board.reserved.size == 0)
        {
            board.reserved = region;
        } else if node.is_compatible(&[b"riscv,plic0", b"sifive,plic-1.0.0"]) && plic.is_none() {
            // Pairs of (interrupt controller, interrupt) per context.
            let contexts = node.prop(b"interrupts-extended").unwrap_or(&[]);
            let context = contexts.chunks_exact(8).position(|pair| be32(pair, 4) == Some(IRQ_S_EXTERNAL as u32));
            plic = reg.zip(context).map(|(reg, context)| (reg.base, context as u64));
        } else if node.name == b"chosen" && node.parent.is_empty() {
            board.stdout = node.prop(b"stdout-path").and_then(unit_address);
            board.root = node.prop(b"bootargs").and_then(root_disk).unwrap_or(0);
        }
    });
    (board.plic, board.plic_context) = plic.ok_or(())?;
    if board.ram_count == 0 {
        return Err(());
    }
    unsafe { *BOARD.get() = board };
    Ok(())
}

fn unit_address(path: &[u8]) -> Option<u64> {
    let at = path.iter().position(|&c| c == b'@')?;
    let digits = path[at + 1..].iter().map_while(|&c| (c as char).to_digit(16));
    Some(digits.fold(0, |value, d| value << 4 | d as u64))
}

fn root_disk(bootargs: &[u8]) -> Option<usize> {
    let root = until_nul(bootargs).split(|&c| c == b' ').find_map(|arg| arg.strip_prefix(b"root=/dev/vd"))?;
    match root {
        [letter @ b'a'..=b'z', ..] => Some((letter - b'a') as usize),
        _ => None,
    }
}

pub fn phys_to_virt(address: u64) -> u64 {
    address.wrapping_add(RAM_VIRTUAL_OFFSET)
}

pub fn virt_to_phys(address: u64) -> u64 {
    address.wrapping_sub(RAM_VIRTUAL_OFFSET)
}
