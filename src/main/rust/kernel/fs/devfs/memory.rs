// SPDX-License-Identifier: MIT

use super::{Device, Name};
use crate::arch::KernelCell;
use crate::fs::Result;
use crate::time::{GOLDEN_GAMMA, entropy, mix64};

pub struct Memory;

impl Device for Memory {
    fn count(&self) -> usize {
        NAMES.len()
    }

    fn name(&self, index: usize) -> Option<Name> {
        Some(Name::of(NAMES[index]))
    }

    fn read(&self, index: usize, _: u64, dst: &mut [u8]) -> Result<usize> {
        match index {
            NULL => return Ok(0),
            ZERO => dst.fill(0),
            _ => RANDOM.with(|state| fill_random(state, dst)),
        }
        Ok(dst.len())
    }

    fn write(&self, _: usize, _: u64, src: &[u8]) -> Result<usize> {
        Ok(src.len())
    }
}

const NAMES: [&[u8]; 4] = [b"null", b"random", b"urandom", b"zero"];
const NULL: usize = 0;
const ZERO: usize = 3;

static RANDOM: KernelCell<u64> = KernelCell::new(0);

fn fill_random(state: &mut u64, dst: &mut [u8]) {
    *state = mix64(*state ^ entropy());
    for chunk in dst.chunks_mut(8) {
        let word = mix64(*state);
        *state = state.wrapping_add(GOLDEN_GAMMA);
        chunk.copy_from_slice(&word.to_le_bytes()[..chunk.len()]);
    }
}
