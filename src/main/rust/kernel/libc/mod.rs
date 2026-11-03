// SPDX-License-Identifier: MIT

pub mod malloc;
pub mod stdio;

use crate::api::{Error, kprintf};
use crate::arch::{RawCell, time_now};
use crate::board::board;
use crate::drivers::rtc;
use crate::power;
use crate::text::Text;
use core::ffi::{c_char, c_int, c_long, c_void};

const EPERM: c_int = 1;
const ENOENT: c_int = 2;
const EINTR: c_int = 4;
const EIO: c_int = 5;
const ENXIO: c_int = 6;
const EBADF: c_int = 9;
const EAGAIN: c_int = 11;
const ENOMEM: c_int = 12;
const EACCES: c_int = 13;
const ENOTBLK: c_int = 15;
const EBUSY: c_int = 16;
const EEXIST: c_int = 17;
const EXDEV: c_int = 18;
const ENODEV: c_int = 19;
const ENOTDIR: c_int = 20;
const EISDIR: c_int = 21;
const EINVAL: c_int = 22;
const EMFILE: c_int = 24;
const EFBIG: c_int = 27;
const ENOSPC: c_int = 28;
const ESPIPE: c_int = 29;
const EROFS: c_int = 30;
const EMLINK: c_int = 31;
const ERANGE: c_int = 34;
const ENAMETOOLONG: c_int = 36;
const ENOSYS: c_int = 38;
const ENOTEMPTY: c_int = 39;
const ELOOP: c_int = 40;
const EOVERFLOW: c_int = 75;
const EILSEQ: c_int = 84;
const ESTALE: c_int = 116;
const ENOMEDIUM: c_int = 123;
const EMEDIUMTYPE: c_int = 124;
const CLOCKS_PER_SEC: u64 = 1_000_000;

static ERRNO: RawCell<c_int> = RawCell::new(0);
static STRERROR: RawCell<Text<32>> = RawCell::new(Text::EMPTY);

pub fn set_errno(value: c_int) {
    unsafe { *ERRNO.get() = value };
}

pub fn errno(e: Error) -> c_int {
    -(e as c_int)
}

#[unsafe(no_mangle)]
extern "C" fn __errno_location() -> *mut c_int {
    ERRNO.get()
}

#[unsafe(no_mangle)]
extern "C" fn ___errno_location() -> *mut c_int {
    ERRNO.get()
}

#[unsafe(no_mangle)]
extern "C" fn __lockfile(_: *mut c_void) -> c_int {
    0
}

#[unsafe(no_mangle)]
extern "C" fn __unlockfile(_: *mut c_void) {}

/// musl's __lock contains a futex system call, for threads.
#[unsafe(no_mangle)]
extern "C" fn __lock(_: *mut c_int) {}

#[unsafe(no_mangle)]
extern "C" fn __unlock(_: *mut c_int) {}

#[unsafe(no_mangle)]
unsafe extern "C" fn __assert_fail(
    expression: *const c_char,
    file: *const c_char,
    line: c_int,
    function: *const c_char,
) -> ! {
    unsafe { kprintf(c"\n*** assertion failed: %s (%s:%d, %s)\n".as_ptr(), expression, file, line, function) };
    power::halt()
}

#[unsafe(no_mangle)]
extern "C" fn abort() -> ! {
    unsafe { kprintf(c"\n*** abort\n".as_ptr()) };
    power::halt()
}

#[unsafe(no_mangle)]
extern "C" fn exit(code: c_int) -> ! {
    stdio::flush_all();
    unsafe { kprintf(c"\n*** exit(%d)\n".as_ptr(), code) };
    power::power_off()
}

#[unsafe(no_mangle)]
extern "C" fn time(out: Option<&mut i64>) -> i64 {
    let now = rtc::seconds() as i64;
    if let Some(out) = out {
        *out = now;
    }
    now
}

#[unsafe(no_mangle)]
extern "C" fn clock() -> c_long {
    (time_now() / (board().timebase / CLOCKS_PER_SEC).max(1)) as c_long
}

/// musl's time zone code maps the zone file. There is none: everything is UTC.
#[unsafe(no_mangle)]
extern "C" fn __map_file(_: *const c_char, _: *mut usize) -> *const u8 {
    core::ptr::null()
}

#[unsafe(no_mangle)]
extern "C" fn __munmap(_: *mut c_void, _: usize) -> c_int {
    0
}

#[unsafe(no_mangle)]
extern "C" fn getenv(_: *const c_char) -> *mut c_char {
    core::ptr::null_mut()
}

#[unsafe(no_mangle)]
extern "C" fn strerror(code: c_int) -> *mut c_char {
    let message = match code {
        0 => c"No error information",
        EPERM => c"Operation not permitted",
        ENOENT => c"No such file or directory",
        EINTR => c"Interrupted system call",
        EIO => c"Input/output error",
        ENXIO => c"No such device or address",
        EBADF => c"Bad file descriptor",
        EAGAIN => c"Resource temporarily unavailable",
        ENOMEM => c"Out of memory",
        EACCES => c"Permission denied",
        ENOTBLK => c"Block device required",
        EBUSY => c"Device or resource busy",
        EEXIST => c"File exists",
        EXDEV => c"Cross-device link",
        ENODEV => c"Unsupported file system",
        ENOTDIR => c"Not a directory",
        EISDIR => c"Is a directory",
        EINVAL => c"Invalid argument",
        EMFILE => c"Too many open files",
        EFBIG => c"File too large",
        ENOSPC => c"No space left on device",
        ESPIPE => c"Illegal seek",
        EROFS => c"Read-only file system",
        EMLINK => c"Too many links",
        ERANGE => c"Result not representable",
        ENAMETOOLONG => c"File name too long",
        ENOSYS => c"Function not implemented",
        ENOTEMPTY => c"Directory not empty",
        ELOOP => c"Too many levels of symbolic links",
        EOVERFLOW => c"Value too large for data type",
        EILSEQ => c"Illegal byte sequence",
        ESTALE => c"Medium changed",
        ENOMEDIUM => c"No medium found",
        EMEDIUMTYPE => c"Medium not formatted",
        _ => {
            let text = unsafe { &mut *STRERROR.get() };
            *text = Text::EMPTY;
            write!(text, "Unknown error {code}");
            return text.as_ptr().cast_mut();
        }
    };
    message.as_ptr().cast_mut()
}

unsafe extern "C" {
    fn strcmp(a: *const c_char, b: *const c_char) -> c_int;
}

#[unsafe(no_mangle)]
unsafe extern "C" fn strcoll(a: *const c_char, b: *const c_char) -> c_int {
    unsafe { strcmp(a, b) }
}

#[unsafe(no_mangle)]
extern "C" fn wcrtomb(dst: *mut c_char, c: i32, _: *mut c_void) -> usize {
    if dst.is_null() {
        return 1;
    }
    if c as u32 >= 0x80 {
        set_errno(EILSEQ);
        return usize::MAX;
    }
    unsafe { *dst = c as c_char };
    1
}

/// For scanf's %lc and %ls, like wcrtomb.
#[unsafe(no_mangle)]
unsafe extern "C" fn mbrtowc(dst: *mut i32, src: *const c_char, len: usize, _: *mut c_void) -> usize {
    if src.is_null() {
        return 0;
    }
    if len == 0 {
        return usize::MAX - 1;
    }
    let c = unsafe { *src };
    if c >= 0x80 {
        set_errno(EILSEQ);
        return usize::MAX;
    }
    if !dst.is_null() {
        unsafe { *dst = c as i32 };
    }
    (c != 0) as usize
}
