// SPDX-License-Identifier: MIT
//! malloc and friends over Matthew Conte's TLSF (external/tlsf). One pool per memory region.

use core::ffi::c_void;
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering::Relaxed};

unsafe extern "C" {
    fn tlsf_create(mem: *mut c_void) -> *mut c_void;
    fn tlsf_add_pool(tlsf: *mut c_void, mem: *mut c_void, bytes: usize) -> *mut c_void;
    fn tlsf_malloc(tlsf: *mut c_void, bytes: usize) -> *mut c_void;
    fn tlsf_realloc(tlsf: *mut c_void, ptr: *mut c_void, size: usize) -> *mut c_void;
    fn tlsf_free(tlsf: *mut c_void, ptr: *mut c_void);
    fn tlsf_block_size(ptr: *mut c_void) -> usize;
    fn tlsf_size() -> usize;
    fn tlsf_block_size_min() -> usize;
    fn tlsf_block_size_max() -> usize;
    fn tlsf_pool_overhead() -> usize;
    fn tlsf_alloc_overhead() -> usize;
}

static TLSF: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
pub(crate) static TOTAL: AtomicUsize = AtomicUsize::new(0); // bytes in pools, without TLSF's overheads
pub(crate) static USED: AtomicUsize = AtomicUsize::new(0); // allocated blocks with their headers

fn is_too_large(size: usize) -> bool {
    size >= unsafe { tlsf_block_size_max() }
}

fn size_on_heap(ptr: *mut c_void) -> usize {
    if ptr.is_null() { 0 } else { unsafe { tlsf_block_size(ptr) + tlsf_alloc_overhead() } }
}

pub unsafe fn add(start: u64, size: u64) {
    let (pool_overhead, block_min) = unsafe { (tlsf_pool_overhead() as u64, tlsf_block_size_min() as u64) };
    let limit = unsafe { tlsf_block_size_max() } as u64 - pool_overhead;
    // TLSF wants 8-byte aligned pools of whole 8-byte words.
    let mut at = (start + 7) & !7;
    let mut size = (size - (at - start)) & !7;
    if TLSF.load(Relaxed).is_null() {
        let control = unsafe { tlsf_size() } as u64;
        if size <= control + pool_overhead + block_min {
            return;
        }
        TLSF.store(unsafe { tlsf_create(at as *mut c_void) }, Relaxed);
        at += control;
        size -= control;
    }
    while size >= pool_overhead + block_min {
        let chunk = size.min(limit);
        if unsafe { tlsf_add_pool(TLSF.load(Relaxed), at as *mut c_void, chunk as usize) }.is_null() {
            return;
        }
        TOTAL.fetch_add((chunk - pool_overhead) as usize, Relaxed);
        at += chunk;
        size -= chunk;
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn malloc(size: usize) -> *mut c_void {
    if size == 0 || is_too_large(size) {
        return core::ptr::null_mut();
    }
    let ptr = unsafe { tlsf_malloc(TLSF.load(Relaxed), size) };
    USED.fetch_add(size_on_heap(ptr), Relaxed);
    ptr
}

#[unsafe(no_mangle)]
extern "C" fn __libc_malloc(size: usize) -> *mut c_void {
    malloc(size)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn free(ptr: *mut c_void) {
    USED.fetch_sub(size_on_heap(ptr), Relaxed);
    unsafe { tlsf_free(TLSF.load(Relaxed), ptr) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    if !ptr.is_null() && size == 0 {
        unsafe { free(ptr) };
        return core::ptr::null_mut();
    }
    if is_too_large(size) {
        return core::ptr::null_mut();
    }
    let before = size_on_heap(ptr);
    let moved = unsafe { tlsf_realloc(TLSF.load(Relaxed), ptr, size) };
    if !moved.is_null() {
        USED.fetch_add(size_on_heap(moved).wrapping_sub(before), Relaxed);
    }
    moved
}

#[unsafe(no_mangle)]
pub extern "C" fn calloc(count: usize, size: usize) -> *mut c_void {
    let Some(bytes) = count.checked_mul(size) else { return core::ptr::null_mut() };
    let ptr = malloc(bytes);
    if !ptr.is_null() {
        unsafe { ptr.cast::<u8>().write_bytes(0, bytes) };
    }
    ptr
}
