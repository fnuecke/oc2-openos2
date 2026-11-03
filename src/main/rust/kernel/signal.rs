// SPDX-License-Identifier: MIT

use crate::api::time::TIME_NEVER;
use crate::arch::KernelCell;
use crate::text::Text;
use crate::time::app_wait;
use core::ffi::CStr;

registry!(pub SOURCES: &'static dyn Source);

pub type Value = Text<64>;

pub trait Source: Sync {
    fn pending(&self) -> bool;

    fn take(&self) -> Option<Event>;

    fn collect(&self) {}

    fn deadline(&self) -> u64 {
        TIME_NEVER
    }
}

#[derive(Clone, Copy)]
pub struct Event {
    pub name: &'static CStr,
    pub value: Option<Value>,
    pub int_values: [i64; 2],
    pub int_value_count: usize,
    pub detail: Option<&'static CStr>,
}

impl Event {
    pub const fn new(name: &'static CStr) -> Self {
        Self { name, value: None, int_values: [0; 2], int_value_count: 0, detail: None }
    }

    pub fn with_value(name: &'static CStr, value: &[u8]) -> Self {
        Self { value: Some(Value::of(value)), ..Self::new(name) }
    }
}

static REMAINING: KernelCell<usize> = KernelCell::new(0);

pub fn wait(deadline: u64) {
    app_wait(pending, SOURCES.iter().map(|source| source.deadline()).fold(deadline, u64::min));
}

pub fn collect() {
    SOURCES.iter().for_each(|source| source.collect());
    REMAINING.with(|remaining| *remaining = SOURCES.len());
}

pub fn next() -> Option<Event> {
    REMAINING.with(|remaining| {
        while *remaining > 0 {
            if let Some(event) = SOURCES[SOURCES.len() - *remaining].take() {
                return Some(event);
            }
            *remaining -= 1;
        }
        None
    })
}

fn pending() -> bool {
    SOURCES.iter().any(|source| source.pending())
}
