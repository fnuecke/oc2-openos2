// SPDX-License-Identifier: MIT

use crate::{kmain, mem};

#[unsafe(no_mangle)]
extern "C" fn stack_usage(peak: &mut usize, size: &mut usize) {
    (*peak, *size) = kmain::stack_usage();
}

#[unsafe(no_mangle)]
extern "C" fn heap_total() -> usize {
    mem::heap_total()
}

#[unsafe(no_mangle)]
extern "C" fn heap_used() -> usize {
    mem::heap_used()
}
