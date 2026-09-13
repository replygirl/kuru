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

const MAX_TREE_DEPTH: usize = 128;

pub(super) fn remove_tree(
    _: &File,
    ancestors: &[(&Path, FileIdentity)],
    path: &Path,
    _: &std::ffi::OsStr,
    held: File,
    expected: FileIdentity,
) -> Result<(), (PublicationPhase, io::Error)> {
    let mut removed = false;
    let _ancestors = pin_ancestors(ancestors, &mut removed)?;
    let root_pin = pin_directory(path, expected, &mut removed)?;
    remove_children(path, root_pin, &mut removed, 0)?;
    remove_empty_directory(path, held, expected).map_err(|(failure, error)| {
        (
            if removed {
                PublicationPhase::Uncertain
            } else {
                failure
            },
            error,
        )
    })?;
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err((PublicationPhase::Uncertain, error)),
        Ok(_) => {
            return Err((
                PublicationPhase::Uncertain,
                denied("removed directory name is still occupied"),
            ));
        }
    }
    Ok(())
}

fn phase(removed: bool) -> PublicationPhase {
    if removed {
        PublicationPhase::Uncertain
    } else {
        PublicationPhase::Rejected
    }
}

fn pin_ancestors(
    ancestors: &[(&Path, FileIdentity)],
    removed: &mut bool,
) -> Result<Vec<File>, (PublicationPhase, io::Error)> {
    ancestors
        .iter()
        .map(|(path, expected)| pin_directory(path, *expected, removed))
        .collect()
}

fn pin_directory(
    path: &Path,
    expected: FileIdentity,
    removed: &mut bool,
) -> Result<File, (PublicationPhase, io::Error)> {
    let current = open(
        path,
        FILE_READ_ATTRIBUTES | READ_CONTROL,
        OPEN_EXISTING,
        FILE_FLAG_BACKUP_SEMANTICS,
        NameRetention::Pinned,
        None,
    )
    .map_err(|error| (phase(*removed), error))?;
    let actual = info(&current).map_err(|error| (phase(*removed), error))?;
    if !actual.directory || actual.file.identity != expected {
        return Err((
            phase(*removed),
            denied("directory entry no longer identifies the held object"),
        ));
    }
    Ok(current)
}

fn remove_children(
    directory: &Path,
    pin: File,
    removed: &mut bool,
    depth: usize,
) -> Result<(), (PublicationPhase, io::Error)> {
    if depth >= MAX_TREE_DEPTH {
        return Err((
            phase(*removed),
            invalid("checked tree removal depth exceeded"),
        ));
    }
    for entry in std::fs::read_dir(directory).map_err(|error| (phase(*removed), error))? {
        let entry = entry.map_err(|error| (phase(*removed), error))?;
        let name = entry.file_name();
        component(&name).map_err(|error| (phase(*removed), error))?;
        let path = entry.path();
        let child_pin = open(
            &path,
            FILE_READ_ATTRIBUTES | READ_CONTROL,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            NameRetention::Pinned,
            None,
        )
        .map_err(|error| (phase(*removed), error))?;
        let child_info = info(&child_pin).map_err(|error| (phase(*removed), error))?;
        let held = open(
            &path,
            FILE_READ_ATTRIBUTES | READ_CONTROL,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            NameRetention::Movable,
            None,
        )
        .map_err(|error| (phase(*removed), error))?;
        let held_info = info(&held).map_err(|error| (phase(*removed), error))?;
        if held_info.file.identity != child_info.file.identity
            || held_info.directory != child_info.directory
        {
            return Err((
                phase(*removed),
                denied("directory entry changed while opening checked removal handle"),
            ));
        }
        if child_info.directory {
            remove_children(&path, child_pin, removed, depth + 1)?;
            remove_empty_directory(&path, held, child_info.file.identity).map_err(
                |(failure, error)| {
                    (
                        if *removed {
                            PublicationPhase::Uncertain
                        } else {
                            failure
                        },
                        error,
                    )
                },
            )?;
        } else {
            drop(child_pin);
            remove_regular(&path, held, child_info.file.identity).map_err(|(failure, error)| {
                (
                    if *removed {
                        PublicationPhase::Uncertain
                    } else {
                        failure
                    },
                    error,
                )
            })?;
        }
        *removed = true;
    }
    drop(pin);
    Ok(())
}

fn remove_empty_directory(
    path: &Path,
    held: File,
    expected: FileIdentity,
) -> Result<(), (PublicationPhase, io::Error)> {
    let held_info = info(&held).map_err(|error| (PublicationPhase::Rejected, error))?;
    if !held_info.directory || held_info.file.identity != expected {
        return Err((
            PublicationPhase::Rejected,
            denied("retained directory no longer has the expected identity"),
        ));
    }
    let deletion = open(
        path,
        DELETE | READ_CONTROL | FILE_READ_ATTRIBUTES,
        OPEN_EXISTING,
        FILE_FLAG_BACKUP_SEMANTICS,
        NameRetention::Movable,
        None,
    )
    .map_err(|error| (PublicationPhase::Rejected, error))?;
    let actual = info(&deletion).map_err(|error| (PublicationPhase::Rejected, error))?;
    if !actual.directory || actual.file.identity != held_info.file.identity {
        return Err((
            PublicationPhase::Rejected,
            denied("directory name no longer identifies the held object"),
        ));
    }
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    // SAFETY: the DELETE-capable handle was opened for this checked directory
    // and compared to the retained full identity before deletion is requested.
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
    drop(deletion);
    drop(held);
    Ok(())
}

fn remove_regular(
    path: &Path,
    held: File,
    expected: FileIdentity,
) -> Result<(), (PublicationPhase, io::Error)> {
    let held_info = info(&held).map_err(|error| (PublicationPhase::Rejected, error))?;
    if held_info.directory || held_info.file.identity != expected {
        return Err((
            PublicationPhase::Rejected,
            denied("retained file no longer has the expected identity"),
        ));
    }
    let deletion = open(
        path,
        DELETE | READ_CONTROL | FILE_READ_ATTRIBUTES,
        OPEN_EXISTING,
        FILE_ATTRIBUTE_NORMAL,
        NameRetention::Movable,
        None,
    )
    .map_err(|error| (PublicationPhase::Rejected, error))?;
    let actual = info(&deletion).map_err(|error| (PublicationPhase::Rejected, error))?;
    if actual.directory || actual.file.identity != held_info.file.identity {
        return Err((
            PublicationPhase::Rejected,
            denied("file name no longer identifies the held object"),
        ));
    }
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    // SAFETY: the DELETE-capable checked regular-file handle remains live for
    // this synchronous disposition request.
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
    use std::fs;
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
    fn checked_tree_removal_rejects_a_junction_without_touching_its_target() {
        let temporary = tempfile::tempdir().unwrap();
        let root_path = temporary.path().join("root");
        let root = Directory::ensure_private(&root_path).unwrap();
        let outside = Directory::ensure_private(&temporary.path().join("outside")).unwrap();
        outside
            .create_new(OsStr::new("sentinel"))
            .unwrap()
            .write_all(b"outside bytes")
            .unwrap();
        let alias = root_path.join("junction");
        junction(&alias, outside.path());

        let error = root.remove_tree().unwrap_err();
        assert_eq!(error.phase, PublicationPhase::Rejected);
        assert!(alias.exists());
        assert_eq!(
            std::fs::read(outside.path().join("sentinel")).unwrap(),
            b"outside bytes"
        );

        std::fs::remove_dir(&alias).unwrap();
        Directory::open(&root_path, Privacy::OwnerOnly, NameRetention::Movable)
            .unwrap()
            .remove_tree()
            .unwrap();
    }

    #[test]
    fn checked_tree_removal_reports_uncertain_after_a_held_root_blocks_final_unlink() {
        let temporary = tempfile::tempdir().unwrap();
        let root_path = temporary.path().join("root");
        let root = Directory::ensure_private(&root_path).unwrap();
        root.create_new(OsStr::new("removed-first"))
            .unwrap()
            .write_all(b"removed before the final directory unlink")
            .unwrap();
        let pinned =
            Directory::open(&root_path, Privacy::OwnerOnly, NameRetention::Pinned).unwrap();

        let error = root.remove_tree().unwrap_err();
        assert_eq!(error.phase, PublicationPhase::Uncertain);
        assert!(root_path.exists());
        assert!(
            !root_path.join("removed-first").exists(),
            "a native final-unlink failure after child removal is uncertain, not retry-safe"
        );

        drop(pinned);
        match Directory::open(&root_path, Privacy::OwnerOnly, NameRetention::Movable) {
            Ok(root) => root.remove_tree().unwrap(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => panic!("held root had an unexpected cleanup result: {error}"),
        }
        assert_eq!(
            std::fs::symlink_metadata(&root_path).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    fn checked_removal_rejects_rebound_names_without_touching_either_identity() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = Directory::ensure_private(&temporary.path().join("private")).unwrap();

        let mut file = directory.create_new(OsStr::new("file")).unwrap();
        file.write_all(b"original file").unwrap();
        drop(file);
        let file_path = directory.path().join("file");
        let held_file = open(
            &file_path,
            FILE_READ_ATTRIBUTES | READ_CONTROL,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            NameRetention::Movable,
            None,
        )
        .unwrap();
        let file_identity = info(&held_file).unwrap().file.identity;
        let parked_file = directory.path().join("parked-file");
        fs::rename(&file_path, &parked_file).unwrap();
        directory
            .create_new(OsStr::new("file"))
            .unwrap()
            .write_all(b"replacement file")
            .unwrap();
        let error = remove_regular(&file_path, held_file, file_identity).unwrap_err();
        assert_eq!(error.0, PublicationPhase::Rejected);
        assert_eq!(fs::read(&parked_file).unwrap(), b"original file");
        assert_eq!(fs::read(&file_path).unwrap(), b"replacement file");

        let dir_path = directory.path().join("child");
        let child = Directory::ensure_private(&dir_path).unwrap();
        child
            .create_new(OsStr::new("original"))
            .unwrap()
            .write_all(b"original directory")
            .unwrap();
        drop(child);
        let held_dir = open(
            &dir_path,
            FILE_READ_ATTRIBUTES | READ_CONTROL,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            NameRetention::Movable,
            None,
        )
        .unwrap();
        let dir_identity = info(&held_dir).unwrap().file.identity;
        let parked_dir = directory.path().join("parked-child");
        fs::rename(&dir_path, &parked_dir).unwrap();
        let replacement = Directory::ensure_private(&dir_path).unwrap();
        replacement
            .create_new(OsStr::new("replacement"))
            .unwrap()
            .write_all(b"replacement directory")
            .unwrap();
        drop(replacement);
        let error = remove_empty_directory(&dir_path, held_dir, dir_identity).unwrap_err();
        assert_eq!(error.0, PublicationPhase::Rejected);
        assert_eq!(
            fs::read(parked_dir.join("original")).unwrap(),
            b"original directory"
        );
        assert_eq!(
            fs::read(dir_path.join("replacement")).unwrap(),
            b"replacement directory"
        );

        let mut removed = false;
        let error = pin_directory(&dir_path, dir_identity, &mut removed).unwrap_err();
        assert_eq!(error.0, PublicationPhase::Rejected);
        assert!(!removed);

        directory.remove_tree().unwrap();
    }

    #[test]
    fn checked_removal_marks_real_late_refusals_uncertain_and_preserves_contents() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = Directory::ensure_private(&temporary.path().join("private")).unwrap();
        let mut file = directory.create_new(OsStr::new("readonly")).unwrap();
        file.write_all(b"readonly bytes").unwrap();
        drop(file);
        let file_path = directory.path().join("readonly");
        let mut permissions = fs::metadata(&file_path).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&file_path, permissions).unwrap();
        let held_file = open(
            &file_path,
            FILE_READ_ATTRIBUTES | READ_CONTROL,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            NameRetention::Movable,
            None,
        )
        .unwrap();
        let file_identity = info(&held_file).unwrap().file.identity;
        let error = remove_regular(&file_path, held_file, file_identity).unwrap_err();
        assert_eq!(error.0, PublicationPhase::Uncertain);
        assert_eq!(fs::read(&file_path).unwrap(), b"readonly bytes");
        assert!(fs::metadata(&file_path).unwrap().permissions().readonly());

        let child_path = directory.path().join("nonempty");
        let child = Directory::ensure_private(&child_path).unwrap();
        drop(child);
        let held_dir = open(
            &child_path,
            FILE_READ_ATTRIBUTES | READ_CONTROL,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            NameRetention::Movable,
            None,
        )
        .unwrap();
        let child_identity = info(&held_dir).unwrap().file.identity;
        fs::write(child_path.join("retained"), b"child bytes").unwrap();
        let error = remove_empty_directory(&child_path, held_dir, child_identity).unwrap_err();
        assert_eq!(error.0, PublicationPhase::Uncertain);
        assert_eq!(
            fs::read(child_path.join("retained")).unwrap(),
            b"child bytes"
        );

        let mut permissions = fs::metadata(&file_path).unwrap().permissions();
        permissions.set_readonly(false);
        fs::set_permissions(&file_path, permissions).unwrap();
        directory.remove_tree().unwrap();
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
