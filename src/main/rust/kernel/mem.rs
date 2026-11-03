// SPDX-License-Identifier: MIT

use crate::libc::malloc::{TOTAL, USED};
pub use crate::libc::malloc::{calloc, free, malloc, realloc};
use core::ops::{Deref, DerefMut};
use core::ptr::NonNull;
use core::sync::atomic::Ordering::Relaxed;

pub struct HeapBox<T>(NonNull<T>);

impl<T> HeapBox<T> {
    pub unsafe fn zeroed() -> Option<Self> {
        NonNull::new(calloc(1, size_of::<T>()).cast()).map(Self)
    }

    /// Keeps it allocated forever, e.g. for memory that a device writes.
    pub fn forget(b: Self) -> *mut T {
        let ptr = b.0.as_ptr();
        core::mem::forget(b);
        ptr
    }
}

impl<T> Deref for HeapBox<T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { self.0.as_ref() }
    }
}

impl<T> DerefMut for HeapBox<T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { self.0.as_mut() }
    }
}

impl<T> Drop for HeapBox<T> {
    fn drop(&mut self) {
        unsafe {
            self.0.drop_in_place();
            free(self.0.as_ptr().cast());
        }
    }
}

pub struct HeapBuf {
    ptr: *mut u8,
    len: usize,
}

impl HeapBuf {
    pub const EMPTY: Self = Self { ptr: core::ptr::null_mut(), len: 0 };

    pub fn new(len: usize) -> Option<Self> {
        Self::of(malloc(len).cast(), len)
    }

    pub fn zeroed(len: usize) -> Option<Self> {
        Self::of(calloc(len, 1).cast(), len)
    }

    pub fn resize(&mut self, len: usize) -> bool {
        if len == 0 {
            *self = Self::EMPTY;
            return true;
        }
        let ptr = unsafe { realloc(self.ptr.cast(), len) }.cast::<u8>();
        if ptr.is_null() {
            return false;
        }
        (self.ptr, self.len) = (ptr, len);
        true
    }

    fn of(ptr: *mut u8, len: usize) -> Option<Self> {
        (!ptr.is_null()).then_some(Self { ptr, len })
    }
}

impl Default for HeapBuf {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl Deref for HeapBuf {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        if self.ptr.is_null() { &[] } else { unsafe { core::slice::from_raw_parts(self.ptr, self.len) } }
    }
}

impl DerefMut for HeapBuf {
    fn deref_mut(&mut self) -> &mut [u8] {
        if self.ptr.is_null() { &mut [] } else { unsafe { core::slice::from_raw_parts_mut(self.ptr, self.len) } }
    }
}

impl Drop for HeapBuf {
    fn drop(&mut self) {
        unsafe { free(self.ptr.cast()) };
    }
}

pub fn heap_total() -> usize {
    TOTAL.load(Relaxed)
}

pub fn heap_used() -> usize {
    USED.load(Relaxed)
}
