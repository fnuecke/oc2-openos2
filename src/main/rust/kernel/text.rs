// SPDX-License-Identifier: MIT

use core::fmt;

#[derive(Clone, Copy)]
pub struct Text<const N: usize> {
    data: [u8; N],
    len: usize,
}

impl<const N: usize> Text<N> {
    pub const EMPTY: Self = Self { data: [0; N], len: 0 };

    pub fn of(s: &[u8]) -> Self {
        let mut text = Self::EMPTY;
        text.push(s);
        text
    }

    pub fn push(&mut self, s: &[u8]) -> &mut Self {
        push(&mut self.data, &mut self.len, s);
        self
    }

    pub fn write_fmt(&mut self, args: fmt::Arguments) {
        let _ = fmt::Write::write_fmt(self, args);
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.data[..self.len]
    }

    /// NUL-terminated.
    pub fn as_ptr(&self) -> *const core::ffi::c_char {
        self.data.as_ptr().cast()
    }
}

impl<const N: usize> fmt::Write for Text<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.push(s.as_bytes());
        Ok(())
    }
}

fn push(data: &mut [u8], len: &mut usize, s: &[u8]) {
    let n = s.len().min(data.len() - 1 - *len);
    data[*len..*len + n].copy_from_slice(&s[..n]);
    *len += n;
    data[*len] = 0;
}
