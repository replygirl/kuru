//! Checked current-image preparation. Never trust `current_exe()` alone after
//! another updater may have rebound that pathname to a different file.

use crate::fs::{Directory, NameRetention, Privacy, regular_file_info};
use std::{
    fs::{File, OpenOptions},
    io,
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::{Path, PathBuf},
    ptr,
};
use windows_sys::Win32::{
    Storage::FileSystem::{
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, GetFinalPathNameByHandleW, VOLUME_NAME_NT,
    },
    System::{
        LibraryLoader::GetModuleHandleW, ProcessStatus::K32GetMappedFileNameW,
        Threading::GetCurrentProcess,
    },
};

/// Keeps the checked file and all pathname ancestors immovable while trusted
/// helper bytes are read. Drop this guard before asking an updater to move the
/// running original. The file is never opened for writing.
pub struct CurrentImage {
    file: File,
    path: PathBuf,
    _parent: Directory,
}

impl CurrentImage {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn file(&self) -> &File {
        &self.file
    }

    pub fn file_mut(&mut self) -> &mut File {
        &mut self.file
    }
}

fn checked_name(mut buffer: Vec<u16>, length: u32) -> io::Result<Vec<u16>> {
    if length == 0 {
        return Err(io::Error::last_os_error());
    }
    let length = length as usize;
    if length >= buffer.len()
        || buffer[length] != 0
        || buffer[..length].contains(&0)
        || !buffer[..length].starts_with(&"\\Device\\".encode_utf16().collect::<Vec<_>>())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "current-image native name is truncated or ambiguous",
        ));
    }
    buffer.truncate(length);
    Ok(buffer)
}

fn mapped_name() -> io::Result<Vec<u16>> {
    // SAFETY: null names select the main executable module, which stays loaded
    // for the entire process lifetime. This is a borrowed module, never freed.
    let module = unsafe { GetModuleHandleW(ptr::null()) };
    if module.is_null() {
        return Err(io::Error::last_os_error());
    }
    let mut buffer = vec![0u16; 32_768];
    // SAFETY: the main-module base belongs to this process, its pseudo handle
    // remains borrowed, and the writable UTF-16 allocation covers nSize.
    let length = unsafe {
        K32GetMappedFileNameW(
            GetCurrentProcess(),
            module.cast(),
            buffer.as_mut_ptr(),
            buffer.len() as u32,
        )
    };
    checked_name(buffer, length)
}

fn file_name(file: &File) -> io::Result<Vec<u16>> {
    let mut buffer = vec![0u16; 32_768];
    // SAFETY: the read-only file remains alive; the bounded writable buffer is
    // measured in UTF-16 code units. NT volume names match the mapping query's
    // namespace without ambient drive-letter or DOS-device resolution.
    let length = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle(),
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            VOLUME_NAME_NT,
        )
    };
    checked_name(buffer, length)
}

/// Obtain the unchanged current-image file, or reject a stale loaded instance.
///
/// The checked file denies both write and delete sharing during both native
/// mapping queries and the caller's copy. Compare exact UTF-16 names: folding
/// case would conflate different files in a case-sensitive Windows directory.
/// The native renamed-and-replaced fixture is an acceptance requirement for
/// this composition; compilation alone does not establish mapping-name rename
/// behavior on a filesystem. Unsupported/ambiguous queries fail closed.
pub fn current_image() -> io::Result<CurrentImage> {
    let path = std::env::current_exe()?;
    let parent = Directory::open(
        path.parent()
            .ok_or_else(|| io::Error::other("current image has no parent"))?,
        Privacy::Inherited,
        NameRetention::Pinned,
    )?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other("current image has no filename"))?;
    let file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&path)?;
    regular_file_info(&file)?;
    parent.verify(name, &file)?;
    let actual = file_name(&file)?;
    if mapped_name()? != actual || file_name(&file)? != actual || mapped_name()? != actual {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "loaded image no longer matches the installed file; restart Kuru before updating",
        ));
    }
    parent.verify(name, &file)?;
    Ok(CurrentImage {
        file,
        path,
        _parent: parent,
    })
}
