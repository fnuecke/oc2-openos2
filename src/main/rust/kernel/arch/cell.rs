// SPDX-License-Identifier: MIT

use core::cell::{RefCell, UnsafeCell};

pub struct IrqCell<T>(UnsafeCell<T>);

unsafe impl<T> Sync for IrqCell<T> {}

impl<T> IrqCell<T> {
    pub const fn new(value: T) -> Self {
        Self(UnsafeCell::new(value))
    }

    pub fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        super::without_irq(|| f(unsafe { &mut *self.0.get() }))
    }
}

pub struct KernelCell<T>(RefCell<T>);

unsafe impl<T> Sync for KernelCell<T> {}

impl<T> KernelCell<T> {
    pub const fn new(value: T) -> Self {
        Self(RefCell::new(value))
    }

    pub fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        f(&mut self.0.borrow_mut())
    }
}

#[repr(transparent)]
pub struct RawCell<T>(UnsafeCell<T>);

unsafe impl<T> Sync for RawCell<T> {}

impl<T> RawCell<T> {
    pub const fn new(value: T) -> Self {
        Self(UnsafeCell::new(value))
    }

    pub const fn get(&self) -> *mut T {
        self.0.get()
    }
}
