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
    pub fn read16(self, offset: u64) -> u16 {
        unsafe { ((self.0 + offset) as *const u16).read_volatile() }
    }

    #[inline(always)]
    pub fn write16(self, offset: u64, value: u16) {
        unsafe { ((self.0 + offset) as *mut u16).write_volatile(value) }
    }

    #[inline(always)]
    pub fn read32(self, offset: u64) -> u32 {
        unsafe { ((self.0 + offset) as *const u32).read_volatile() }
    }

    #[inline(always)]
    pub fn write32(self, offset: u64, value: u32) {
        unsafe { ((self.0 + offset) as *mut u32).write_volatile(value) }
    }

    #[inline(always)]
    pub fn read64(self, offset: u64) -> u64 {
        unsafe { ((self.0 + offset) as *const u64).read_volatile() }
    }

    #[inline(always)]
    pub fn write64(self, offset: u64, value: u64) {
        unsafe { ((self.0 + offset) as *mut u64).write_volatile(value) }
    }
}
