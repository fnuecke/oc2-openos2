// SPDX-License-Identifier: MIT

use super::{EINVAL, ENOMEM, ESPIPE, errno, set_errno};
use crate::api::fs::{O_APPEND, O_CREATE, O_READ, O_TRUNCATE, O_WRITE};
use crate::api::time::TIME_NEVER;
use crate::api::{buffer, buffer_mut, bytes};
use crate::drivers::{console, uart};
use crate::fs::vfs::{self, Fd};
use crate::fs::{Error, OpenFlags, Result};
use crate::mem::calloc;
use crate::time::{self, entropy, mix64};
use core::ffi::{c_char, c_int, c_long};
use core::mem::offset_of;

const EOF: c_int = -1;
const UNGET: usize = 8;
const BUFSIZ: usize = 1024;
const F_PERM: u32 = 1;
const F_NORD: u32 = 4;
const F_NOWR: u32 = 8;
const F_EOF: u32 = 16;
const F_ERR: u32 = 32;
const F_APP: u32 = 128;

/// musl 1.2.4's struct _IO_FILE, see its src/internal/stdio_impl.h.
#[repr(C)]
pub struct File {
    flags: u32,
    rpos: *mut u8,
    rend: *mut u8,
    close: Option<unsafe extern "C" fn(*mut File) -> c_int>,
    wend: *mut u8,
    wpos: *mut u8,
    _mustbezero_1: *mut u8,
    wbase: *mut u8,
    read: Option<unsafe extern "C" fn(*mut File, *mut u8, usize) -> usize>,
    write: Option<unsafe extern "C" fn(*mut File, *const u8, usize) -> usize>,
    seek: Option<unsafe extern "C" fn(*mut File, i64, c_int) -> i64>,
    buf: *mut u8,
    buf_size: usize,
    _list: [usize; 2], // prev, next
    fd: c_int,
    _pipe_pid: c_int,
    _lockcount: c_long,
    mode: c_int,
    lock: c_int,
    lbf: c_int,
    _rest: [usize; 10], // cookie, off, getln_buf, mustbezero_2, shend, shlim, shcnt, prev_locked, next_locked, locale
}

const _: () = assert!(size_of::<File>() == 232 && offset_of!(File, fd) == 120 && offset_of!(File, lock) == 140);

unsafe extern "C" {
    static stdin: *mut File;
    static stdout: *mut File;
    fn __ofl_add(f: *mut File) -> *mut File;
    fn fclose(f: *mut File) -> c_int;
    fn fflush(f: *mut File) -> c_int;
    fn getc_unlocked(f: *mut File) -> c_int;
    fn putc_unlocked(c: c_int, f: *mut File) -> c_int;
}

pub fn flush_all() {
    unsafe { fflush(core::ptr::null_mut()) };
}

#[unsafe(no_mangle)]
unsafe extern "C" fn __stdio_read(f: *mut File, dst: *mut u8, len: usize) -> usize {
    unsafe { read(&mut *f, dst, len, console_read) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn __stdio_write(f: *mut File, src: *const u8, len: usize) -> usize {
    unsafe { write(&mut *f, src, len, console_write) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn __stdout_write(f: *mut File, src: *const u8, len: usize) -> usize {
    unsafe { __stdio_write(f, src, len) }
}

#[unsafe(no_mangle)]
extern "C" fn __stdio_seek(_: *mut File, _: i64, _: c_int) -> i64 {
    set_errno(ESPIPE);
    -1
}

#[unsafe(no_mangle)]
extern "C" fn __stdio_close(_: *mut File) -> c_int {
    0
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fopen(path: *const c_char, mode: *const c_char) -> *mut File {
    let Some((flags, _)) = (unsafe { parse_mode(mode) }) else {
        set_errno(EINVAL);
        return core::ptr::null_mut();
    };
    unsafe { open_file(vfs::open(bytes(path), flags), mode) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fdopen(fd: c_int, mode: *const c_char) -> *mut File {
    let Some((_, flags)) = (unsafe { parse_mode(mode) }) else {
        set_errno(EINVAL);
        return core::ptr::null_mut();
    };
    let f = calloc(1, size_of::<File>() + UNGET + BUFSIZ).cast::<File>();
    if f.is_null() {
        set_errno(ENOMEM);
        return core::ptr::null_mut();
    }
    let file = unsafe { &mut *f };
    file.buf = unsafe { f.add(1).cast::<u8>().add(UNGET) };
    file.buf_size = BUFSIZ;
    file.lbf = EOF;
    init(file, fd, flags);
    unsafe { __ofl_add(f) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn freopen(path: *const c_char, mode: *const c_char, f: *mut File) -> *mut File {
    unsafe { fflush(f) };
    let opened = match unsafe { parse_mode(mode) } {
        Some((flags, file_flags)) if !path.is_null() => {
            vfs::open(unsafe { bytes(path) }, flags).map(|fd| (fd, file_flags))
        }
        _ => Err(Error::Invalid),
    };
    let (fd, flags) = match opened {
        Ok(opened) => opened,
        Err(e) => {
            unsafe { fclose(f) };
            set_errno(errno(e));
            return core::ptr::null_mut();
        }
    };
    unsafe { (*f).close.unwrap()(f) };
    let file = unsafe { &mut *f };
    init(file, fd.0 as c_int, flags | (file.flags & F_PERM));
    f
}

#[unsafe(no_mangle)]
extern "C" fn tmpfile() -> *mut File {
    unsafe { open_file(vfs::open_anonymous(), c"w+".as_ptr()) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn __mkostemps(template: *mut c_char, suffix: c_int, _flags: c_int) -> c_int {
    const LETTERS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let len = unsafe { bytes(template) }.len();
    let at = usize::try_from(suffix).ok().and_then(|suffix| len.checked_sub(suffix + 6));
    let Some(name) = at.map(|at| unsafe { template.cast::<u8>().add(at) }) else { return fail(Error::Invalid) };
    if unsafe { buffer(name, 6) } != b"XXXXXX" {
        return fail(Error::Invalid);
    }
    let mut random = entropy();
    let mut error = Error::Exists;
    for _ in 0..100 {
        random = mix64(random);
        let text: [u8; 6] = core::array::from_fn(|i| LETTERS[(random >> (i * 8)) as usize % LETTERS.len()]);
        unsafe { name.copy_from_nonoverlapping(text.as_ptr(), 6) };
        let path = unsafe { bytes(template) };
        match vfs::stat(path) {
            Ok(_) => continue,
            Err(Error::NotFound) => match vfs::open(path, OpenFlags(O_READ | O_WRITE | O_CREATE)) {
                Ok(fd) => return fd.0 as c_int,
                Err(e) => error = e,
            },
            Err(e) => error = e,
        }
        break;
    }
    unsafe { name.copy_from_nonoverlapping(b"XXXXXX".as_ptr(), 6) };
    fail(error)
}

#[unsafe(no_mangle)]
unsafe extern "C" fn mkostemps(template: *mut c_char, suffix: c_int, flags: c_int) -> c_int {
    unsafe { __mkostemps(template, suffix, flags) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn remove(path: *const c_char) -> c_int {
    vfs::remove(unsafe { bytes(path) }).map_or_else(fail, |_| 0)
}

#[unsafe(no_mangle)]
unsafe extern "C" fn rename(from: *const c_char, to: *const c_char) -> c_int {
    vfs::rename(unsafe { bytes(from) }, unsafe { bytes(to) }).map_or_else(fail, |_| 0)
}

// Avoid thread pointer read in musl's getc, putc and the like.

#[unsafe(no_mangle)]
unsafe extern "C" fn getc(f: *mut File) -> c_int {
    unsafe { getc_unlocked(f) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fgetc(f: *mut File) -> c_int {
    unsafe { getc_unlocked(f) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn getchar() -> c_int {
    unsafe { getc_unlocked(stdin) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn putc(c: c_int, f: *mut File) -> c_int {
    unsafe { putc_unlocked(c, f) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn fputc(c: c_int, f: *mut File) -> c_int {
    unsafe { putc_unlocked(c, f) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn putchar(c: c_int) -> c_int {
    unsafe { putc_unlocked(c, stdout) }
}

unsafe extern "C" fn file_read(f: *mut File, dst: *mut u8, len: usize) -> usize {
    let f = unsafe { &mut *f };
    let fd = Fd(f.fd as usize);
    unsafe { read(f, dst, len, |dst| vfs::read(fd, dst)) }
}

unsafe extern "C" fn file_write(f: *mut File, src: *const u8, len: usize) -> usize {
    let f = unsafe { &mut *f };
    let fd = Fd(f.fd as usize);
    unsafe { write(f, src, len, |src| vfs::write(fd, src)) }
}

unsafe extern "C" fn file_seek(f: *mut File, offset: i64, whence: c_int) -> i64 {
    vfs::seek(Fd(unsafe { (*f).fd } as usize), offset, whence).map_or_else(|e| fail(e) as i64, |at| at as i64)
}

unsafe extern "C" fn file_close(f: *mut File) -> c_int {
    vfs::close(Fd(unsafe { (*f).fd } as usize)).map_or_else(fail, |_| 0)
}

unsafe fn open_file(opened: Result<Fd>, mode: *const c_char) -> *mut File {
    match opened {
        Ok(fd) => {
            let f = unsafe { fdopen(fd.0 as c_int, mode) };
            if f.is_null() {
                let _ = vfs::close(fd);
            }
            f
        }
        Err(e) => {
            set_errno(errno(e));
            core::ptr::null_mut()
        }
    }
}

unsafe fn parse_mode(mode: *const c_char) -> Option<(OpenFlags, u32)> {
    let mode = unsafe { bytes(mode) };
    let (open, flags) = match mode.first()? {
        b'r' => (O_READ, F_NOWR),
        b'w' => (O_WRITE | O_CREATE | O_TRUNCATE, F_NORD),
        b'a' => (O_WRITE | O_CREATE | O_APPEND, F_NORD | F_APP),
        _ => return None,
    };
    if mode.contains(&b'x') {
        return None;
    }
    if mode.contains(&b'+') {
        return Some((OpenFlags(open | O_READ | O_WRITE), flags & F_APP));
    }
    Some((OpenFlags(open), flags))
}

fn init(f: &mut File, fd: c_int, flags: u32) {
    f.flags = flags;
    f.fd = fd;
    f.rpos = core::ptr::null_mut();
    f.rend = core::ptr::null_mut();
    f.wpos = core::ptr::null_mut();
    f.wbase = core::ptr::null_mut();
    f.wend = core::ptr::null_mut();
    f.read = Some(file_read);
    f.write = Some(file_write);
    f.seek = Some(file_seek);
    f.close = Some(file_close);
    f.mode = 0;
    f.lock = -1;
}

unsafe fn read(f: &mut File, dst: *mut u8, len: usize, source: impl FnOnce(&mut [u8]) -> Result<usize>) -> usize {
    let direct = len >= f.buf_size;
    let target = unsafe { if direct { buffer_mut(dst, len) } else { buffer_mut(f.buf, f.buf_size) } };
    match source(target) {
        Ok(0) => {
            f.flags |= F_EOF;
            0
        }
        Err(e) => {
            f.flags |= F_ERR;
            set_errno(errno(e));
            0
        }
        Ok(count) if direct => count,
        Ok(count) => {
            let copied = count.min(len);
            unsafe {
                dst.copy_from_nonoverlapping(f.buf, copied);
                f.rpos = f.buf.add(copied);
                f.rend = f.buf.add(count);
            }
            copied
        }
    }
}

unsafe fn write(f: &mut File, src: *const u8, len: usize, mut sink: impl FnMut(&[u8]) -> Result<usize>) -> usize {
    let pending = unsafe { buffer(f.wbase, f.wpos.addr() - f.wbase.addr()) };
    let data = unsafe { buffer(src, len) };
    match write_all(pending, &mut sink).and_then(|_| write_all(data, &mut sink)) {
        Ok(()) => {
            unsafe { f.wend = f.buf.add(f.buf_size) };
            f.wpos = f.buf;
            f.wbase = f.buf;
            len
        }
        Err(e) => {
            f.wpos = core::ptr::null_mut();
            f.wbase = core::ptr::null_mut();
            f.wend = core::ptr::null_mut();
            f.flags |= F_ERR;
            set_errno(errno(e));
            0
        }
    }
}

fn write_all(mut data: &[u8], sink: &mut impl FnMut(&[u8]) -> Result<usize>) -> Result<()> {
    while !data.is_empty() {
        match sink(data)? {
            0 => return Err(Error::NoSpace),
            count => data = &data[count..],
        }
    }
    Ok(())
}

fn console_read(dst: &mut [u8]) -> Result<usize> {
    time::app_wait(|| uart::readable(uart::console()), TIME_NEVER);
    match uart::read(uart::console(), dst) {
        Err(Error::Again) => Ok(0),
        result => result,
    }
}

fn console_write(src: &[u8]) -> Result<usize> {
    console::write(src);
    Ok(src.len())
}

fn fail(e: Error) -> c_int {
    set_errno(errno(e));
    -1
}
