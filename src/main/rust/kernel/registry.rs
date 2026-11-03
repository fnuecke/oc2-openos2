// SPDX-License-Identifier: MIT

use core::ops::Deref;

pub struct Registry<T: 'static> {
    start: *const T,
    end: *const T,
}

unsafe impl<T: Sync> Sync for Registry<T> {}

impl<T> Registry<T> {
    pub const unsafe fn new(start: *const u8, end: *const u8) -> Self {
        Self { start: start.cast(), end: end.cast() }
    }
}

impl<T> Deref for Registry<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        let length = (self.end as usize - self.start as usize) / size_of::<T>();
        unsafe { core::slice::from_raw_parts(self.start, length) }
    }
}

macro_rules! registry {
    ($vis:vis $name:ident: $ty:ty) => {
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        $vis type $name = $ty;
        $vis static $name: $crate::registry::Registry<$ty> = {
            unsafe extern "C" {
                #[link_name = concat!("__registry_", stringify!($name), "_start")]
                static START: [u8; 0];
                #[link_name = concat!("__registry_", stringify!($name), "_end")]
                static END: [u8; 0];
            }
            unsafe { $crate::registry::Registry::new(&raw const START as *const u8, &raw const END as *const u8) }
        };
        const _: () = {
            #[used]
            #[unsafe(export_name = concat!("__registry_", stringify!($name)))]
            static UNIQUE: u8 = 0;
        };
    };
}

macro_rules! register {
    ($name:ident, $value:expr) => {
        const _: () = {
            #[used]
            #[unsafe(link_section = concat!(".registry.", stringify!($name)))]
            static ENTRY: $name = $value;
        };
    };
}
