// SPDX-License-Identifier: MIT

use super::{Device, MAX_FILES, Name, lookup};
use crate::arch::KernelCell;
use crate::drivers::virtio::blk::{self, Fault, SECTOR_SIZE};
use crate::fs::{Data, Error, OpenFlags, Result, ext2, trim_slashes};

pub struct Disks;

impl Device for Disks {
    fn count(&self) -> usize {
        blk::count().min(26)
    }

    fn name(&self, index: usize) -> Option<Name> {
        Some(Name::of(&[b'v', b'd', b'a' + index as u8]))
    }

    fn open(&self, index: usize, flags: OpenFlags) -> Result<usize> {
        if flags.has(OpenFlags::APPEND) {
            return Err(Error::NoSpace);
        }
        let handle = CLAIMS.with(|claims| claims.iter().position(Option::is_none)).ok_or(Error::TooMany)?;
        let write = flags.has(OpenFlags::WRITE);
        let generation = ext2::claim(index, write)?;
        CLAIMS.with(|claims| claims[handle] = Some(Claim { device: index, generation, write }));
        Ok(handle)
    }

    fn read(&self, handle: usize, pos: u64, dst: &mut [u8]) -> Result<usize> {
        transfer(claim(handle)?, pos, Data::Read(dst))
    }

    fn write(&self, handle: usize, pos: u64, src: &[u8]) -> Result<usize> {
        let claim = claim(handle)?;
        if !claim.write {
            return Err(Error::BadFd);
        }
        match transfer(claim, pos, Data::Write(src))? {
            0 if !src.is_empty() => Err(Error::NoSpace),
            count => Ok(count),
        }
    }

    fn seekable(&self) -> bool {
        true
    }

    fn size(&self, handle: usize) -> Result<u64> {
        let claim = claim(handle)?;
        claim.check()?;
        Ok(claim.disk().sectors() * SECTOR_SIZE as u64)
    }

    fn close(&self, handle: usize) -> Result<()> {
        let claim = CLAIMS.with(|claims| claims[handle].take()).ok_or(Error::Invalid)?;
        ext2::release(claim.device, claim.write, claim.generation);
        Ok(())
    }
}

const CHUNK: usize = 0x10000;

static CLAIMS: KernelCell<[Option<Claim>; MAX_FILES]> = KernelCell::new([None; MAX_FILES]);
static SECTOR: KernelCell<[u8; SECTOR_SIZE]> = KernelCell::new([0; SECTOR_SIZE]);

pub fn disk(path: &[u8]) -> Result<usize> {
    match Disks.find(trim_slashes(path)) {
        Some(disk) => Ok(disk),
        None => lookup(path).and(Err(Error::NotBlock)),
    }
}

fn claim(handle: usize) -> Result<Claim> {
    CLAIMS.with(|claims| claims[handle]).ok_or(Error::Invalid)
}

fn transfer(claim: Claim, pos: u64, mut data: Data) -> Result<usize> {
    claim.check()?;
    let capacity = claim.disk().sectors() * SECTOR_SIZE as u64;
    let len = (data.len() as u64).min(capacity.saturating_sub(pos)) as usize;
    let mut done = 0;
    while done < len {
        let at = pos + done as u64;
        let sector = at / SECTOR_SIZE as u64;
        let offset = (at % SECTOR_SIZE as u64) as usize;
        let result = if offset == 0 && len - done >= SECTOR_SIZE {
            let count = ((len - done) & !(SECTOR_SIZE - 1)).min(CHUNK);
            claim.transfer(sector, data.slice(done, count)).map(|_| count)
        } else {
            let count = (SECTOR_SIZE - offset).min(len - done);
            SECTOR.with(|buf| {
                claim.transfer(sector, Data::Read(buf))?;
                match &mut data {
                    Data::Read(dst) => dst[done..done + count].copy_from_slice(&buf[offset..offset + count]),
                    Data::Write(src) => {
                        buf[offset..offset + count].copy_from_slice(&src[done..done + count]);
                        claim.transfer(sector, Data::Write(buf))?;
                    }
                }
                Ok(count)
            })
        };
        match result {
            Ok(count) => done += count,
            Err(e) if done == 0 => return Err(e),
            Err(_) => break,
        }
    }
    Ok(done)
}

#[derive(Clone, Copy)]
struct Claim {
    device: usize,
    generation: u32,
    write: bool,
}

impl Claim {
    fn disk(&self) -> blk::Disk {
        blk::get(self.device).expect("a claimed device exists")
    }

    fn check(&self) -> Result<()> {
        if ext2::generation(self.device) == Some(self.generation) { Ok(()) } else { Err(Error::Changed) }
    }

    fn transfer(&self, sector: u64, mut data: Data) -> Result<()> {
        loop {
            self.check()?;
            let result = match &mut data {
                Data::Read(dst) => self.disk().read(sector, dst),
                Data::Write(src) => self.disk().write(sector, src),
            };
            match result {
                Ok(()) => return Ok(()),
                Err(Fault::Io) => return Err(Error::Io),
                Err(Fault::Changed) => ext2::poll_media(),
            }
        }
    }
}
