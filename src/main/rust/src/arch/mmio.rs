// SPDX-License-Identifier: MIT

#[derive(Clone, Copy)]
pub struct Mmio(u64);

impl Mmio {
    pub const unsafe fn new(base: u64) -> Self {
        Self(base)
    }

    pub fn base(self) -> u64 {
        self.0
    }

    #[inline(always)]
    pub fn read8(self, offset: u64) -> u8 {
        unsafe { ((self.0 + offset) as *const u8).read_volatile() }
    }

    #[inline(always)]
    pub fn write8(self, offset: u64, value: u8) {
        unsafe { ((self.0 + offset) as *mut u8).write_volatile(value) }
    }

    #[inline(always)]
    pub fn read32(self, offset: u64) -> u32 {
        unsafe { ((self.0 + offset) as *const u32).read_volatile() }
    }

    #[inline(always)]
    pub fn write32(self, offset: u64, value: u32) {
        unsafe { ((self.0 + offset) as *mut u32).write_volatile(value) }
    }
}
