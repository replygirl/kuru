//! Audited Win32 filesystem interop. All public filesystem entrypoints are safe.
#![allow(unsafe_code)]

use super::*;
use crate::windows::security::{self, PrivateSecurity};
use std::mem::size_of;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::io::{AsHandle, AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Prefix;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CREATE_NEW, CreateDirectoryW, CreateFileW, DELETE, FILE_ALL_ACCESS, FILE_ATTRIBUTE_NORMAL,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO, FILE_DISPOSITION_INFO,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_EXECUTE,
    FILE_GENERIC_READ, FILE_ID_INFO, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, FILE_STANDARD_INFO, FileAttributeTagInfo, FileDispositionInfo, FileIdInfo,
    FileStandardInfo, GetFileInformationByHandleEx, GetVolumeInformationByHandleW,
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW, OPEN_ALWAYS, OPEN_EXISTING,
    READ_CONTROL, SetFileInformationByHandle, WRITE_DAC,
};
use windows_sys::Win32::System::SystemServices::FILE_PERSISTENT_ACLS;

pub(super) fn normalize(path: &Path) -> io::Result<PathBuf> {
    match path.components().next() {
        Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_)) => {
            // Ordinary Win32 paths accept either separator. Our internal \\?\
            // prefix disables that conversion, so perform only this lossless
            // UTF-16 substitution before entering the extended-path API.
            let value: Vec<_> = path
                .as_os_str()
                .encode_wide()
                .map(|unit| {
                    if unit == u16::from(b'/') {
                        u16::from(b'\\')
                    } else {
                        unit
                    }
                })
                .collect();
            Ok(std::ffi::OsString::from_wide(&value).into())
        }
        Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::VerbatimDisk(_)) => {
            if path
                .as_os_str()
                .encode_wide()
                .any(|unit| unit == u16::from(b'/'))
            {
                return Err(invalid(
                    "forward slash in explicit verbatim filesystem path",
                ));
            }
            Ok(path.to_path_buf())
        }
        _ => Err(invalid(
            "checked filesystem requires an absolute local Windows volume path",
        )),
    }
}

fn wide(path: &Path) -> io::Result<Vec<u16>> {
    let path = normalize(path)?;
    let mut value: Vec<_> = path.as_os_str().encode_wide().collect();
    if value.contains(&0) {
        return Err(invalid("NUL in native filesystem path"));
    }
    if !matches!(path.components().next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::VerbatimDisk(_)))
    {
        let mut extended: Vec<_> = "\\\\?\\".encode_utf16().collect();
        extended.extend(value);
        value = extended;
    }
    value.push(0);
    Ok(value)
}

/// `T` must be the exact native record for `kind`, accepting every returned bit
/// pattern; no reference-bearing or otherwise constrained Rust type is valid.
unsafe fn query<T: Default>(file: &File, kind: i32) -> io::Result<T> {
    let mut value = T::default();
    // SAFETY: callers pair each information class with its exact native output
    // structure; the file handle and properly aligned output remain live.
    let status = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            kind,
            (&mut value as *mut T).cast(),
            size_of::<T>() as u32,
        )
    };
    if status == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(value)
    }
}

pub(super) fn info(file: &File) -> io::Result<ObjectInfo> {
    // SAFETY: each class below is paired with its exact plain native record.
    let attributes: FILE_ATTRIBUTE_TAG_INFO = unsafe { query(file, FileAttributeTagInfo)? };
    if attributes.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(denied("reparse points are not accepted filesystem objects"));
    }
    // SAFETY: FILE_STANDARD_INFO is the documented output for FileStandardInfo.
    let standard: FILE_STANDARD_INFO = unsafe { query(file, FileStandardInfo)? };
    if standard.DeletePending || standard.EndOfFile < 0 {
        return Err(denied(
            "filesystem object is pending deletion or has an invalid size",
        ));
    }
    // SAFETY: FILE_ID_INFO retains the complete documented 128-bit identity.
    let identity: FILE_ID_INFO = unsafe { query(file, FileIdInfo)? };
    Ok(ObjectInfo {
        file: FileInfo {
            identity: FileIdentity {
                volume: identity.VolumeSerialNumber,
                object: identity.FileId.Identifier,
            },
            links: u64::from(standard.NumberOfLinks),
            len: standard.EndOfFile as u64,
        },
        directory: standard.Directory,
    })
}

fn persistent_acls(file: &File) -> io::Result<()> {
    let mut flags = 0;
    // SAFETY: the held handle identifies the volume; only the writable flags
    // output is requested, so optional buffers are null with zero lengths.
    let status = unsafe {
        GetVolumeInformationByHandleW(
            file.as_raw_handle(),
            null_mut(),
            0,
            null_mut(),
            null_mut(),
            &mut flags,
            null_mut(),
            0,
        )
    };
    if status == 0 {
        return Err(io::Error::last_os_error());
    }
    if flags & FILE_PERSISTENT_ACLS == 0 {
        return Err(denied(
            "private storage requires a filesystem enforcing persistent ACLs",
        ));
    }
    Ok(())
}

pub(super) fn require_private(file: &File) -> io::Result<()> {
    persistent_acls(file)?;
    security::require_private(file.as_handle(), false)
}

pub(super) fn private_chain(files: &[&File]) -> io::Result<()> {
    persistent_acls(
        files
            .last()
            .ok_or_else(|| invalid("missing private root"))?,
    )?;
    for file in files.iter().rev() {
        if security::private_status(file.as_handle())? {
            return Ok(());
        }
    }
    Err(denied(
        "owner-only descendants require a protected private ancestor",
    ))
}

fn open(
    path: &Path,
    access: u32,
    creation: u32,
    flags: u32,
    retention: NameRetention,
    security: Option<&PrivateSecurity>,
) -> io::Result<File> {
    let path = wide(path)?;
    let attributes = security.map(PrivateSecurity::attributes);
    let mut share = FILE_SHARE_READ | FILE_SHARE_WRITE;
    if retention == NameRetention::Movable {
        share |= FILE_SHARE_DELETE;
    }
    // SAFETY: path is NUL-terminated UTF-16; optional attributes borrow a live
    // descriptor through this call. Inheritance is explicitly disabled. A
    // successful returned HANDLE receives exactly one RAII owner below.
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            access,
            share,
            attributes
                .as_ref()
                .map_or(null(), |value| value as *const _),
            creation,
            flags | FILE_FLAG_OPEN_REPARSE_POINT,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreateFileW success transfers unique handle ownership.
    let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
    Ok(File::from(handle))
}

pub(super) fn open_directory(
    _: Option<&File>,
    path: &Path,
    retention: NameRetention,
) -> io::Result<File> {
    open(
        path,
        FILE_READ_ATTRIBUTES | READ_CONTROL,
        OPEN_EXISTING,
        FILE_FLAG_BACKUP_SEMANTICS,
        retention,
        None,
    )
}

pub(super) fn create_directory(parent: &File, path: &Path) -> io::Result<()> {
    persistent_acls(parent)?;
    let security = PrivateSecurity::new(FILE_ALL_ACCESS, true)?;
    let attributes = security.attributes();
    let path = wide(path)?;
    // SAFETY: path/descriptor/attributes remain live, attributes deny handle
    // inheritance and install the protected ACL atomically with creation.
    if unsafe { CreateDirectoryW(path.as_ptr(), &attributes) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(super) fn open_file(
    _: &File,
    path: &Path,
    mode: OpenMode,
    privacy: Privacy,
    retention: NameRetention,
) -> io::Result<File> {
    let (mut access, creation) = match mode {
        OpenMode::Read => (GENERIC_READ | READ_CONTROL, OPEN_EXISTING),
        OpenMode::ReadWrite => (GENERIC_READ | GENERIC_WRITE | READ_CONTROL, OPEN_EXISTING),
        OpenMode::New => (GENERIC_READ | GENERIC_WRITE | READ_CONTROL, CREATE_NEW),
        OpenMode::Lock => (GENERIC_READ | GENERIC_WRITE | READ_CONTROL, OPEN_ALWAYS),
    };
    if privacy == Privacy::OwnerOnly && !matches!(mode, OpenMode::Read) {
        access |= WRITE_DAC;
    }
    let security =
        if privacy == Privacy::OwnerOnly && matches!(mode, OpenMode::New | OpenMode::Lock) {
            Some(PrivateSecurity::new(FILE_ALL_ACCESS, false)?)
        } else {
            None
        };
    open(
        path,
        access,
        creation,
        FILE_ATTRIBUTE_NORMAL,
        retention,
        security.as_ref(),
    )
}

pub(super) fn seal_private(file: &File, executable: bool) -> io::Result<()> {
    let access = FILE_GENERIC_READ | if executable { FILE_GENERIC_EXECUTE } else { 0 };
    security::set_private(file.as_handle(), access)
}

pub(super) fn make_executable(_: &File) -> io::Result<()> {
    // Windows has no Unix executable mode; preserve the installed file's ACL.
    // Type/hash/version checks are separate from its extension and access policy.
    Ok(())
}

pub(super) fn remove(
    _: &File,
    path: &Path,
    held: File,
) -> Result<(), (PublicationPhase, io::Error)> {
    let deletion = (|| -> io::Result<File> {
        let deletion = open(
            path,
            DELETE | READ_CONTROL | FILE_READ_ATTRIBUTES,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            NameRetention::Movable,
            None,
        )?;
        if checked_file(&deletion)?.identity != checked_file(&held)?.identity {
            return Err(denied("removal name no longer identifies the held object"));
        }
        Ok(deletion)
    })()
    .map_err(|error| (PublicationPhase::Rejected, error))?;
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    // SAFETY: this DELETE-capable handle was checked against the retained full
    // file identity. The class matches this exact live native structure. Use
    // ordinary image-section checks, never POSIX unlink or ignored READONLY.
    let status = unsafe {
        SetFileInformationByHandle(
            deletion.as_raw_handle(),
            FileDispositionInfo,
            (&disposition as *const FILE_DISPOSITION_INFO).cast(),
            size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    };
    if status == 0 {
        return Err((PublicationPhase::Uncertain, io::Error::last_os_error()));
    }
    // Neither our deletion handle nor the caller's old read handle may retain
    // a delete-pending object when the shared API checks actual disappearance.
    drop(deletion);
    drop(held);
    Ok(())
}

pub(super) fn publish(
    _: &File,
    source: &Path,
    _: &File,
    destination: &Path,
    policy: Publication,
) -> Result<(), (PublicationPhase, io::Error)> {
    let source = wide(source).map_err(|error| (PublicationPhase::Rejected, error))?;
    let destination = wide(destination).map_err(|error| (PublicationPhase::Rejected, error))?;
    let flags = MOVEFILE_WRITE_THROUGH
        | if policy == Publication::ReplaceRegular {
            MOVEFILE_REPLACE_EXISTING
        } else {
            0
        };
    // SAFETY: both paths are live, validated local names. Same-volume checks
    // happened before this call; COPY_ALLOWED is deliberately absent. Native
    // write-through publication replaces Unix parent-directory fsync here.
    if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), flags) } == 0 {
        // Conservatively retain uncertainty once Windows attempted publication;
        // callers inspect held identity/receipts and never infer a rollback.
        Err((PublicationPhase::Uncertain, io::Error::last_os_error()))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use windows_sys::Win32::System::IO::DeviceIoControl;
    use windows_sys::Win32::System::Ioctl::FSCTL_SET_REPARSE_POINT;

    #[test]
    fn separator_conversion_preserves_raw_utf16_and_explicit_verbatim_policy() {
        let ordinary = [67, 58, 92, 0xd800, 47, 0xdc00, 92, 65];
        let path = PathBuf::from(std::ffi::OsString::from_wide(&ordinary));
        let normalized: Vec<_> = normalize(&path)
            .unwrap()
            .as_os_str()
            .encode_wide()
            .collect();
        assert_eq!(normalized, [67, 58, 92, 0xd800, 92, 0xdc00, 92, 65]);
        let mut extended: Vec<_> = "\\\\?\\".encode_utf16().collect();
        extended.extend(&normalized);
        let expected = PathBuf::from(std::ffi::OsString::from_wide(&extended));
        assert_eq!(normalize(&expected).unwrap(), expected);
        extended.push(0);
        assert_eq!(wide(&path).unwrap(), extended);
        let mut invalid: Vec<_> = "\\\\?\\".encode_utf16().collect();
        invalid.extend(ordinary);
        assert!(normalize(Path::new(&std::ffi::OsString::from_wide(&invalid))).is_err());
    }
    use windows_sys::Win32::System::SystemServices::IO_REPARSE_TAG_MOUNT_POINT;

    fn junction(path: &Path, destination: &Path) {
        std::fs::create_dir(path).unwrap();
        let file = open(
            path,
            GENERIC_WRITE | READ_CONTROL,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            NameRetention::Movable,
            None,
        )
        .unwrap();
        let native = wide(destination).unwrap();
        let print = native[4..native.len() - 1].to_vec();
        let substitute: Vec<_> = "\\??\\"
            .encode_utf16()
            .chain(print.iter().copied())
            .collect();
        let substitute_bytes = u16::try_from(substitute.len() * 2).unwrap();
        let print_bytes = u16::try_from(print.len() * 2).unwrap();
        let data_bytes = 8 + substitute_bytes + 2 + print_bytes + 2;
        let mut buffer = Vec::new();
        buffer.extend_from_slice(&IO_REPARSE_TAG_MOUNT_POINT.to_le_bytes());
        buffer.extend_from_slice(&data_bytes.to_le_bytes());
        buffer.extend_from_slice(&0u16.to_le_bytes());
        buffer.extend_from_slice(&0u16.to_le_bytes());
        buffer.extend_from_slice(&substitute_bytes.to_le_bytes());
        buffer.extend_from_slice(&(substitute_bytes + 2).to_le_bytes());
        buffer.extend_from_slice(&print_bytes.to_le_bytes());
        for character in substitute
            .into_iter()
            .chain(Some(0))
            .chain(print)
            .chain(Some(0))
        {
            buffer.extend_from_slice(&character.to_le_bytes());
        }
        let mut returned = 0;
        // SAFETY: this isolated empty fixture directory is uniquely owned by the
        // test. The bounded mount-point buffer follows REPARSE_DATA_BUFFER's
        // documented byte layout, and synchronous DeviceIoControl retains no
        // pointers. No external path or live user directory is modified.
        assert_ne!(
            unsafe {
                DeviceIoControl(
                    file.as_raw_handle(),
                    FSCTL_SET_REPARSE_POINT,
                    buffer.as_ptr().cast(),
                    buffer.len() as u32,
                    null_mut(),
                    0,
                    &mut returned,
                    null_mut(),
                )
            },
            0,
            "create native junction fixture: {}",
            io::Error::last_os_error()
        );
    }

    #[test]
    fn real_junctions_are_rejected_at_the_leaf_and_in_ancestors() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = Directory::ensure_private(&temporary.path().join("private")).unwrap();
        let outside = Directory::ensure_private(&temporary.path().join("outside")).unwrap();
        outside
            .create_new(OsStr::new("sentinel"))
            .unwrap()
            .write_all(b"outside bytes")
            .unwrap();
        let alias = directory.path().join("junction");
        junction(&alias, outside.path());
        assert!(directory.read(OsStr::new("junction")).is_err());
        assert!(directory.lock_file(OsStr::new("junction")).is_err());
        assert!(Directory::open(&alias, Privacy::OwnerOnly, NameRetention::Movable).is_err());
        assert!(Directory::ensure_private(&alias.join("new-child")).is_err());
        assert!(!outside.path().join("new-child").exists());
        assert_eq!(
            std::fs::read(outside.path().join("sentinel")).unwrap(),
            b"outside bytes"
        );
        std::fs::remove_dir(&alias).unwrap();
        assert!(outside.path().join("sentinel").is_file());
    }

    #[test]
    fn a_real_conflicting_handle_preserves_old_bytes_on_native_replace_failure() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = Directory::ensure_private(&temporary.path().join("private")).unwrap();
        directory
            .create_new(OsStr::new("published"))
            .unwrap()
            .write_all(b"old")
            .unwrap();
        let mut candidate = directory.create_new(OsStr::new("candidate")).unwrap();
        candidate.write_all(b"new").unwrap();
        let pinned =
            Directory::open(directory.path(), Privacy::OwnerOnly, NameRetention::Pinned).unwrap();
        let held = pinned.read(OsStr::new("published")).unwrap();
        let error = directory
            .publish_file(
                &directory,
                OsStr::new("candidate"),
                &candidate,
                OsStr::new("published"),
                Publication::ReplaceRegular,
            )
            .unwrap_err();
        assert_eq!(error.phase, PublicationPhase::Uncertain);
        assert_eq!(std::fs::read(&error.destination).unwrap(), b"old");
        assert_eq!(
            std::fs::read(directory.path().join("candidate")).unwrap(),
            b"new"
        );
        drop(held);
        drop(pinned);
        directory
            .publish_file(
                &directory,
                OsStr::new("candidate"),
                &candidate,
                OsStr::new("published"),
                Publication::ReplaceRegular,
            )
            .unwrap();
        assert_eq!(
            std::fs::read(directory.path().join("published")).unwrap(),
            b"new"
        );
    }
}
