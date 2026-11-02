// SPDX-License-Identifier: MIT

/// Negative errno values, described by strerror(-error).
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    NotPermitted = -1,  // EPERM
    NotFound = -2,      // ENOENT
    Interrupted = -4,   // EINTR
    Io = -5,            // EIO
    NoAddress = -6,     // ENXIO
    BadFd = -9,         // EBADF
    Again = -11,        // EAGAIN
    Denied = -13,       // EACCES
    NotBlock = -15,     // ENOTBLK
    Busy = -16,         // EBUSY
    Exists = -17,       // EEXIST
    CrossDevice = -18,  // EXDEV
    Unsupported = -19,  // ENODEV
    NotDir = -20,       // ENOTDIR
    IsDir = -21,        // EISDIR
    Invalid = -22,      // EINVAL
    TooMany = -24,      // EMFILE
    FileTooBig = -27,   // EFBIG
    NoSpace = -28,      // ENOSPC
    ReadOnly = -30,     // EROFS
    TooManyLinks = -31, // EMLINK
    NameTooLong = -36,  // ENAMETOOLONG
    NotEmpty = -39,     // ENOTEMPTY
    Loop = -40,         // ELOOP
    Changed = -116,     // ESTALE
    NoMedium = -123,    // ENOMEDIUM
    Unformatted = -124, // EMEDIUMTYPE
}

impl Error {
    const ALL: [Error; 27] = [
        Error::NotPermitted,
        Error::NotFound,
        Error::Interrupted,
        Error::Io,
        Error::NoAddress,
        Error::BadFd,
        Error::Again,
        Error::Denied,
        Error::NotBlock,
        Error::Busy,
        Error::Exists,
        Error::CrossDevice,
        Error::Unsupported,
        Error::NotDir,
        Error::IsDir,
        Error::Invalid,
        Error::TooMany,
        Error::FileTooBig,
        Error::NoSpace,
        Error::ReadOnly,
        Error::TooManyLinks,
        Error::NameTooLong,
        Error::NotEmpty,
        Error::Loop,
        Error::Changed,
        Error::NoMedium,
        Error::Unformatted,
    ];

    /// The variant for an errno, if there is one.
    pub fn from_errno(number: u32) -> Option<Error> {
        Self::ALL.into_iter().find(|&e| -(e as i64) == number as i64)
    }
}

pub type Result<T> = core::result::Result<T, Error>;
