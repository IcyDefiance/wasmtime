//! WASI host types specific to Windows host.
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(unused)]
use crate::old::snapshot_0::host::FileType;
use crate::old::snapshot_0::{error::FromRawOsError, wasi, Error, Result};
use std::convert::TryInto;
use std::ffi::OsStr;
use std::fs::OpenOptions;
use std::fs::{self, File};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::time::{SystemTime, UNIX_EPOCH};
use winx::file::{AccessMode, Attributes, CreationDisposition, Flags};
use winx::winerror::WinError;

impl FromRawOsError for Error {
    fn from_raw_os_error(code: i32) -> Self {
        Self::from(WinError::from_u32(code as u32))
    }
}

impl From<WinError> for Error {
    fn from(err: WinError) -> Self {
        // TODO: implement error mapping between Windows and WASI
        use winx::winerror::WinError::*;
        match err {
            ERROR_SUCCESS => Self::ESUCCESS,
            ERROR_BAD_ENVIRONMENT => Self::E2BIG,
            ERROR_FILE_NOT_FOUND => Self::ENOENT,
            ERROR_PATH_NOT_FOUND => Self::ENOENT,
            ERROR_TOO_MANY_OPEN_FILES => Self::ENFILE,
            ERROR_ACCESS_DENIED => Self::EACCES,
            ERROR_SHARING_VIOLATION => Self::EACCES,
            ERROR_PRIVILEGE_NOT_HELD => Self::ENOTCAPABLE, // TODO is this the correct mapping?
            ERROR_INVALID_HANDLE => Self::EBADF,
            ERROR_INVALID_NAME => Self::ENOENT,
            ERROR_NOT_ENOUGH_MEMORY => Self::ENOMEM,
            ERROR_OUTOFMEMORY => Self::ENOMEM,
            ERROR_DIR_NOT_EMPTY => Self::ENOTEMPTY,
            ERROR_NOT_READY => Self::EBUSY,
            ERROR_BUSY => Self::EBUSY,
            ERROR_NOT_SUPPORTED => Self::ENOTSUP,
            ERROR_FILE_EXISTS => Self::EEXIST,
            ERROR_BROKEN_PIPE => Self::EPIPE,
            ERROR_BUFFER_OVERFLOW => Self::ENAMETOOLONG,
            ERROR_NOT_A_REPARSE_POINT => Self::EINVAL,
            ERROR_NEGATIVE_SEEK => Self::EINVAL,
            ERROR_DIRECTORY => Self::ENOTDIR,
            ERROR_ALREADY_EXISTS => Self::EEXIST,
            _ => Self::ENOTSUP,
        }
    }
}

pub(crate) fn fdflags_from_win(mode: AccessMode) -> wasi::__wasi_fdflags_t {
    let mut fdflags = 0;
    // TODO verify this!
    if mode.contains(AccessMode::FILE_APPEND_DATA) {
        fdflags |= wasi::__WASI_FDFLAGS_APPEND;
    }
    if mode.contains(AccessMode::SYNCHRONIZE) {
        fdflags |= wasi::__WASI_FDFLAGS_DSYNC;
        fdflags |= wasi::__WASI_FDFLAGS_RSYNC;
        fdflags |= wasi::__WASI_FDFLAGS_SYNC;
    }
    // The NONBLOCK equivalent is FILE_FLAG_OVERLAPPED
    // but it seems winapi doesn't provide a mechanism
    // for checking whether the handle supports async IO.
    // On the contrary, I've found some dicsussion online
    // which suggests that on Windows all handles should
    // generally be assumed to be opened with async support
    // and then the program should fallback should that **not**
    // be the case at the time of the operation.
    // TODO: this requires further investigation
    fdflags
}

pub(crate) fn win_from_fdflags(fdflags: wasi::__wasi_fdflags_t) -> (AccessMode, Flags) {
    let mut access_mode = AccessMode::empty();
    let mut flags = Flags::empty();

    // TODO verify this!
    if fdflags & wasi::__WASI_FDFLAGS_NONBLOCK != 0 {
        flags.insert(Flags::FILE_FLAG_OVERLAPPED);
    }
    if fdflags & wasi::__WASI_FDFLAGS_APPEND != 0 {
        access_mode.insert(AccessMode::FILE_APPEND_DATA);
    }
    if fdflags & wasi::__WASI_FDFLAGS_DSYNC != 0
        || fdflags & wasi::__WASI_FDFLAGS_RSYNC != 0
        || fdflags & wasi::__WASI_FDFLAGS_SYNC != 0
    {
        access_mode.insert(AccessMode::SYNCHRONIZE);
    }

    (access_mode, flags)
}

pub(crate) fn win_from_oflags(oflags: wasi::__wasi_oflags_t) -> CreationDisposition {
    if oflags & wasi::__WASI_OFLAGS_CREAT != 0 {
        if oflags & wasi::__WASI_OFLAGS_EXCL != 0 {
            CreationDisposition::CREATE_NEW
        } else {
            CreationDisposition::CREATE_ALWAYS
        }
    } else if oflags & wasi::__WASI_OFLAGS_TRUNC != 0 {
        CreationDisposition::TRUNCATE_EXISTING
    } else {
        CreationDisposition::OPEN_EXISTING
    }
}

pub(crate) fn filetype_from_std(ftype: &fs::FileType) -> FileType {
    if ftype.is_file() {
        FileType::RegularFile
    } else if ftype.is_dir() {
        FileType::Directory
    } else if ftype.is_symlink() {
        FileType::Symlink
    } else {
        FileType::Unknown
    }
}

fn num_hardlinks(file: &File) -> io::Result<u64> {
    Ok(winx::file::get_fileinfo(file)?.nNumberOfLinks.into())
}

fn device_id(file: &File) -> io::Result<u64> {
    Ok(winx::file::get_fileinfo(file)?.dwVolumeSerialNumber.into())
}

pub(crate) fn file_serial_no(file: &File) -> io::Result<u64> {
    let info = winx::file::get_fileinfo(file)?;
    let high = info.nFileIndexHigh;
    let low = info.nFileIndexLow;
    let no = (u64::from(high) << 32) | u64::from(low);
    Ok(no)
}

fn change_time(file: &File) -> io::Result<i64> {
    winx::file::change_time(file)
}

fn systemtime_to_timestamp(st: SystemTime) -> Result<u64> {
    st.duration_since(UNIX_EPOCH)
        .map_err(|_| Error::EINVAL)? // date earlier than UNIX_EPOCH
        .as_nanos()
        .try_into()
        .map_err(Into::into) // u128 doesn't fit into u64
}

pub(crate) fn filestat_from_win(file: &File) -> Result<wasi::__wasi_filestat_t> {
    let metadata = file.metadata()?;
    Ok(wasi::__wasi_filestat_t {
        dev: device_id(file)?,
        ino: file_serial_no(file)?,
        nlink: num_hardlinks(file)?.try_into()?, // u64 doesn't fit into u32
        size: metadata.len(),
        atim: systemtime_to_timestamp(metadata.accessed()?)?,
        ctim: change_time(file)?.try_into()?, // i64 doesn't fit into u64
        mtim: systemtime_to_timestamp(metadata.modified()?)?,
        filetype: filetype_from_std(&metadata.file_type()).to_wasi(),
    })
}

/// Creates owned WASI path from OS string.
///
/// NB WASI spec requires OS string to be valid UTF-8. Otherwise,
/// `__WASI_ERRNO_ILSEQ` error is returned.
pub(crate) fn path_from_host<S: AsRef<OsStr>>(s: S) -> Result<String> {
    let vec: Vec<u16> = s.as_ref().encode_wide().collect();
    String::from_utf16(&vec).map_err(|_| Error::EILSEQ)
}
