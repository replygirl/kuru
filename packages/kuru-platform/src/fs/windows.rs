//! Audited Win32 filesystem interop. All public filesystem entrypoints are safe.
#![allow(unsafe_code)]

use super::*;
use crate::windows::security::{self, PrivateSecurity};
use std::mem::size_of;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::io::{AsHandle, AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Prefix;
use std::ptr::{null, null_mut};
use windows_sys::Wdk::Storage::FileSystem::{FileRenameInformationEx, NtSetInformationFile};
use windows_sys::Win32::Foundation::{
    GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE, RtlNtStatusToDosError, STATUS_PENDING,
    WAIT_OBJECT_0,
};
use windows_sys::Win32::Storage::FileSystem::{
    CREATE_NEW, CreateDirectoryW, CreateFileW, DELETE, FILE_ADD_FILE, FILE_ALL_ACCESS,
    FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO,
    FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_ID_INFO, FILE_READ_ATTRIBUTES, FILE_RENAME_INFO,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_STANDARD_INFO, FileAttributeTagInfo,
    FileDispositionInfo, FileIdInfo, FileStandardInfo, GetFileInformationByHandleEx,
    GetVolumeInformationByHandleW, MOVEFILE_WRITE_THROUGH, MoveFileExW, OPEN_ALWAYS, OPEN_EXISTING,
    READ_CONTROL, ReOpenFile, SYNCHRONIZE, SetFileInformationByHandle, WRITE_DAC,
};
use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;
use windows_sys::Win32::System::SystemServices::FILE_PERSISTENT_ACLS;
use windows_sys::Win32::System::Threading::{INFINITE, WaitForSingleObject};
use windows_sys::Win32::System::WindowsProgramming::{
    FILE_RENAME_FLAG_POSIX_SEMANTICS, FILE_RENAME_FLAG_REPLACE_IF_EXISTS,
};

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

fn inspect_info(file: &File, allow_retained_delete_pending: bool) -> io::Result<ObjectInfo> {
    // SAFETY: each class below is paired with its exact plain native record.
    let attributes: FILE_ATTRIBUTE_TAG_INFO = unsafe { query(file, FileAttributeTagInfo)? };
    if attributes.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(denied("reparse points are not accepted filesystem objects"));
    }
    // SAFETY: FILE_STANDARD_INFO is the documented output for FileStandardInfo.
    let standard: FILE_STANDARD_INFO = unsafe { query(file, FileStandardInfo)? };
    if (standard.DeletePending && (!allow_retained_delete_pending || standard.NumberOfLinks != 0))
        || standard.EndOfFile < 0
    {
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

pub(super) fn info(file: &File) -> io::Result<ObjectInfo> {
    inspect_info(file, false)
}

pub(super) fn retained_info(file: &File) -> io::Result<ObjectInfo> {
    // POSIX replacement marks the old object for deletion while its already
    // held, now-unlinked handle remains readable. Ordinary admission still
    // uses strict info, and a delete-pending object with a live link is refused.
    inspect_info(file, true)
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

pub(super) fn copy_file_access(source: &File, staged: &File) -> io::Result<()> {
    // The original read handle includes READ_CONTROL; owner-private newly
    // created stages retain WRITE_DAC. Keep the source ACL's protected bit.
    staged.set_permissions(source.metadata()?.permissions())?;
    security::copy_file_dacl(source.as_handle(), staged.as_handle())
}

pub(super) fn prepare_file_replacement(staged: &File) -> io::Result<Option<File>> {
    // Capture DELETE while this exact staged object still carries its private
    // creation DACL. A later copied ordinary DACL may omit file DELETE while
    // the destination parent still authorizes replacement through DELETE_CHILD.
    // SAFETY: ReOpenFile derives a new handle from the retained exact object.
    let replacement = unsafe {
        ReOpenFile(
            staged.as_raw_handle(),
            DELETE | SYNCHRONIZE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_FLAG_OPEN_REPARSE_POINT,
        )
    };
    if replacement == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful ReOpenFile transfers one unique handle owner.
    let replacement = unsafe { OwnedHandle::from_raw_handle(replacement) };
    Ok(Some(File::from(replacement)))
}

pub(super) fn file_access_token(source: &File) -> io::Result<Vec<u8>> {
    security::file_access_token(source.as_handle())
}

pub(super) fn retired_access_matches(
    source: &File,
    before: &[u8],
    after: &[u8],
) -> io::Result<bool> {
    // This exception cannot be used for a named or still-linked source. The
    // caller has already checked this held file's full identity and metadata;
    // the kernel's delete-pending bit proves it is the displaced old object.
    // SAFETY: FileStandardInfo returns the plain FILE_STANDARD_INFO record.
    let standard: FILE_STANDARD_INFO = unsafe { query(source, FileStandardInfo)? };
    Ok(standard.DeletePending
        && standard.NumberOfLinks == 0
        && security::same_retired_file_access(before, after))
}

pub(super) fn finalize_file_access(source: &File, published: &File) -> io::Result<()> {
    security::restore_file_dacl_inheritance(source.as_handle(), published.as_handle())
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
    // A root that is already absent is the outcome this removal wants, not a
    // rejection: an earlier attempt, or the holder of a pending delete, got
    // there first. The absence check below still proves the outcome.
    match pin_directory(path, expected, &mut removed) {
        Ok(root_pin) => {
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
        }
        Err((_, error)) if error.kind() == io::ErrorKind::NotFound => drop(held),
        Err(failure) => return Err(failure),
    }
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

/// Map an already-absent target to a completed step; every other failure keeps
/// its phase and error.
fn absent_is_done<T>(
    result: io::Result<T>,
    removed: bool,
) -> Result<Option<T>, (PublicationPhase, io::Error)> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err((phase(removed), error)),
    }
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
        #[cfg(test)]
        super::enumeration_seam::observe();
        // An enumerated name already gone by the time this removal opens it -
        // a delete-pending entry whose last handle closed after the
        // enumeration - is the outcome this loop wants, not a rejection.
        // `removed` stays untouched: we performed no removal here, and the
        // final absence check still proves the tree's disappearance.
        let Some(child_pin) = absent_is_done(
            open(
                &path,
                FILE_READ_ATTRIBUTES | READ_CONTROL,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                NameRetention::Pinned,
                None,
            ),
            *removed,
        )?
        else {
            continue;
        };
        let Some(child_info) = absent_is_done(info(&child_pin), *removed)? else {
            continue;
        };
        let Some(held) = absent_is_done(
            open(
                &path,
                FILE_READ_ATTRIBUTES | READ_CONTROL,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                NameRetention::Movable,
                None,
            ),
            *removed,
        )?
        else {
            continue;
        };
        let Some(held_info) = absent_is_done(info(&held), *removed)? else {
            continue;
        };
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
    // A directory name that is already absent needs no deletion; that is this
    // removal's outcome, not a rejection.
    let deletion = match open(
        path,
        DELETE | READ_CONTROL | FILE_READ_ATTRIBUTES,
        OPEN_EXISTING,
        FILE_FLAG_BACKUP_SEMANTICS,
        NameRetention::Movable,
        None,
    ) {
        Ok(deletion) => deletion,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err((PublicationPhase::Rejected, error)),
    };
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
    source_file: &File,
    prepared_replacement: Option<&File>,
    destination_parent: &File,
    destination: &Path,
    policy: Publication,
) -> Result<(), (PublicationPhase, io::Error)> {
    if policy == Publication::ReplaceRegular {
        return replace_open_destination(
            destination_parent,
            source_file,
            prepared_replacement,
            destination,
        );
    }
    let source = wide(source).map_err(|error| (PublicationPhase::Rejected, error))?;
    let destination = wide(destination).map_err(|error| (PublicationPhase::Rejected, error))?;
    let flags = MOVEFILE_WRITE_THROUGH;
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

fn replace_open_destination(
    destination_parent: &File,
    source: &File,
    prepared_replacement: Option<&File>,
    destination: &Path,
) -> Result<(), (PublicationPhase, io::Error)> {
    let expected_parent =
        info(destination_parent).map_err(|error| (PublicationPhase::Rejected, error))?;
    if !expected_parent.directory {
        return Err((
            PublicationPhase::Rejected,
            denied("publication parent is not a directory"),
        ));
    }
    // The checked inspection handle has read rights only. Acquire creation
    // authority through a fresh directory open, then bind that short-lived
    // handle to the still-retained exact parent before it can authorize any
    // rename. A path rebound or reparse point is rejected before mutation.
    let parent_path = destination.parent().ok_or_else(|| {
        (
            PublicationPhase::Rejected,
            invalid("missing publication parent path"),
        )
    })?;
    let publication_parent = open(
        parent_path,
        FILE_ADD_FILE | SYNCHRONIZE | FILE_READ_ATTRIBUTES,
        OPEN_EXISTING,
        FILE_FLAG_BACKUP_SEMANTICS,
        NameRetention::Movable,
        None,
    )
    .map_err(|error| (PublicationPhase::Rejected, error))?;
    let actual_parent =
        info(&publication_parent).map_err(|error| (PublicationPhase::Rejected, error))?;
    if !actual_parent.directory || actual_parent.file.identity != expected_parent.file.identity {
        return Err((
            PublicationPhase::Rejected,
            denied("publication parent no longer has the retained identity"),
        ));
    }
    let name = destination.file_name().ok_or_else(|| {
        (
            PublicationPhase::Rejected,
            invalid("missing publication filename"),
        )
    })?;
    let name: Vec<_> = name.encode_wide().collect();
    if name.is_empty() || name.contains(&0) {
        return Err((
            PublicationPhase::Rejected,
            invalid("invalid native publication filename"),
        ));
    }
    let name_bytes = name
        .len()
        .checked_mul(size_of::<u16>())
        .and_then(|bytes| u32::try_from(bytes).ok())
        .ok_or_else(|| {
            (
                PublicationPhase::Rejected,
                invalid("native publication filename exceeds bound"),
            )
        })?;
    // FileRenameInformationEx requires the full structure, including its trailing
    // FileName placeholder, plus the UTF-16 bytes named by FileNameLength.
    let record_bytes = size_of::<FILE_RENAME_INFO>()
        .checked_add(name_bytes as usize)
        .ok_or_else(|| {
            (
                PublicationPhase::Rejected,
                invalid("native publication record exceeds bound"),
            )
        })?;
    let record_len = u32::try_from(record_bytes).map_err(|_| {
        (
            PublicationPhase::Rejected,
            invalid("native publication record exceeds bound"),
        )
    })?;
    let mut record = vec![0usize; record_bytes.div_ceil(size_of::<usize>())];
    let record_ptr = record.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    // SAFETY: the usize-backed buffer is aligned for FILE_RENAME_INFO and is
    // sized through the complete variable-length UTF-16 FileName field.
    unsafe {
        (*record_ptr).Anonymous.Flags =
            FILE_RENAME_FLAG_REPLACE_IF_EXISTS | FILE_RENAME_FLAG_POSIX_SEMANTICS;
        (*record_ptr).RootDirectory = publication_parent.as_raw_handle();
        (*record_ptr).FileNameLength = name_bytes;
        std::ptr::copy_nonoverlapping(
            name.as_ptr(),
            record
                .as_mut_ptr()
                .cast::<u8>()
                .add(std::mem::offset_of!(FILE_RENAME_INFO, FileName)) as *mut u16,
            name.len(),
        );
    }
    let reopened = prepared_replacement
        .is_none()
        .then(|| prepare_file_replacement(source))
        .transpose()
        .map_err(|error| (PublicationPhase::Rejected, error))?
        .flatten();
    let renamed = prepared_replacement
        .or(reopened.as_ref())
        .expect("Windows replacement handle exists");
    let mut io_status = IO_STATUS_BLOCK::default();
    io_status.Anonymous.Status = STATUS_PENDING;
    // SAFETY: the source, aligned record, status block and retained exact
    // destination-parent handle remain live until authoritative completion.
    // The same record and rights passed the isolated native Windows fixture;
    // SetFileInformationByHandle(FileRenameInfoEx) rejected that fixture.
    let mut status = unsafe {
        NtSetInformationFile(
            renamed.as_raw_handle(),
            &mut io_status,
            record.as_mut_ptr().cast(),
            record_len,
            FileRenameInformationEx,
        )
    };
    if status == STATUS_PENDING {
        // ReOpenFile omitted FILE_FLAG_OVERLAPPED and retained SYNCHRONIZE.
        // Pending is unexpected for that synchronous file object, but the
        // caller cannot reconcile from a snapshot while its rename may still
        // complete. Keep every kernel-referenced input alive and wait for the
        // final IO_STATUS_BLOCK, just as the synchronous open normally does.
        loop {
            let waited = unsafe { WaitForSingleObject(renamed.as_raw_handle(), INFINITE) };
            if waited == WAIT_OBJECT_0 {
                // SAFETY: a completed file-object wait publishes IO_STATUS_BLOCK.
                status = unsafe { io_status.Anonymous.Status };
                if status != STATUS_PENDING {
                    break;
                }
            }
            // A spurious signal or failed wait still cannot release borrowed
            // storage or permit a concurrent retry; avoid a busy loop.
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    if status == 0 {
        Ok(())
    } else {
        // Keep the NTSTATUS as well as its Win32 mapping for exact diagnosis.
        let mapped = unsafe { RtlNtStatusToDosError(status) };
        Err((
            PublicationPhase::Uncertain,
            io::Error::new(
                io::Error::from_raw_os_error(mapped as i32).kind(),
                format!("native replacement NTSTATUS {status:#010x} (Win32 {mapped})"),
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::{Read, Write};
    use windows_sys::Win32::Storage::FileSystem::{FILE_LIST_DIRECTORY, FILE_TRAVERSE};
    use windows_sys::Win32::System::IO::DeviceIoControl;
    use windows_sys::Win32::System::Ioctl::FSCTL_SET_REPARSE_POINT;

    #[test]
    fn retained_directory_rights_distinguish_delete_open_from_path_rename() {
        let temporary = tempfile::tempdir().unwrap();
        let cases = [
            ("metadata", FILE_READ_ATTRIBUTES | READ_CONTROL),
            (
                "traverse",
                FILE_READ_ATTRIBUTES | READ_CONTROL | FILE_TRAVERSE,
            ),
            (
                "list",
                FILE_READ_ATTRIBUTES | READ_CONTROL | FILE_LIST_DIRECTORY,
            ),
        ];
        for (name, access) in cases {
            let delete_path = temporary.path().join(format!("{name}-delete"));
            fs::create_dir(&delete_path).unwrap();
            fs::write(delete_path.join("item"), b"original").unwrap();
            let retained = open(
                &delete_path,
                access,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                NameRetention::Pinned,
                None,
            );
            let retained = match retained {
                Ok(retained) => retained,
                Err(error) => {
                    assert_ne!(name, "metadata", "the production access must open: {error}");
                    writeln!(
                        io::stderr(),
                        "pinned_directory_rights case={name} open={:?}",
                        error.raw_os_error()
                    )
                    .unwrap();
                    continue;
                }
            };
            let identity = info(&retained).unwrap().file.identity;
            let delete_result = open(
                &delete_path,
                DELETE,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                NameRetention::Movable,
                None,
            );
            let delete_code = delete_result
                .as_ref()
                .err()
                .and_then(io::Error::raw_os_error);
            drop(delete_result);
            assert_eq!(info(&retained).unwrap().file.identity, identity);
            assert_eq!(fs::read(delete_path.join("item")).unwrap(), b"original");
            writeln!(
                io::stderr(),
                "pinned_directory_rights case={name} open=ok delete_open={delete_code:?} delete_identity=true delete_bytes=true"
            )
            .unwrap();
            drop(retained);
            let unpinned_delete = open(
                &delete_path,
                DELETE,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                NameRetention::Movable,
                None,
            )
            .expect("same-path DELETE open must succeed without the retained handle");
            drop(unpinned_delete);
            writeln!(
                io::stderr(),
                "pinned_directory_rights case={name} unpinned_delete=ok"
            )
            .unwrap();

            // Keep the rename independent of the DELETE-open attempt so one
            // probe cannot alter the sharing state observed by the other.
            let rename_path = temporary.path().join(format!("{name}-rename"));
            let moved_path = temporary.path().join(format!("{name}-moved"));
            fs::create_dir(&rename_path).unwrap();
            fs::write(rename_path.join("item"), b"original").unwrap();
            let retained = open(
                &rename_path,
                access,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                NameRetention::Pinned,
                None,
            )
            .unwrap();
            let identity = info(&retained).unwrap().file.identity;
            let rename_result = fs::rename(&rename_path, &moved_path);
            let rename_code = rename_result
                .as_ref()
                .err()
                .and_then(io::Error::raw_os_error);
            let renamed_with_retained_handle = rename_result.is_ok();
            writeln!(
                io::stderr(),
                "pinned_directory_rights case={name} rename={rename_code:?}"
            )
            .unwrap();
            match rename_result {
                Ok(()) => {
                    let moved =
                        Directory::open(&moved_path, Privacy::Inherited, NameRetention::Movable)
                            .unwrap();
                    assert_eq!(moved.identity(), identity);
                    assert_eq!(fs::read(moved_path.join("item")).unwrap(), b"original");
                    assert!(!rename_path.exists());
                    fs::create_dir(&rename_path).unwrap();
                    fs::write(rename_path.join("replacement"), b"replacement").unwrap();
                    let replacement =
                        Directory::open(&rename_path, Privacy::Inherited, NameRetention::Movable)
                            .unwrap();
                    assert_ne!(replacement.identity(), identity);
                    assert_eq!(
                        fs::read(rename_path.join("replacement")).unwrap(),
                        b"replacement"
                    );
                }
                Err(_) => {
                    let original =
                        Directory::open(&rename_path, Privacy::Inherited, NameRetention::Movable)
                            .unwrap();
                    assert_eq!(original.identity(), identity);
                    assert_eq!(fs::read(rename_path.join("item")).unwrap(), b"original");
                    assert!(!moved_path.exists());
                }
            }
            assert_eq!(info(&retained).unwrap().file.identity, identity);
            drop(retained);
            if !renamed_with_retained_handle {
                fs::rename(&rename_path, &moved_path)
                    .expect("same-path rename must succeed without the retained handle");
                let moved =
                    Directory::open(&moved_path, Privacy::Inherited, NameRetention::Movable)
                        .unwrap();
                assert_eq!(moved.identity(), identity);
                assert_eq!(fs::read(moved_path.join("item")).unwrap(), b"original");
            }
            writeln!(
                io::stderr(),
                "pinned_directory_rights case={name} rename_identity=true rename_bytes=true unpinned_rename={}",
                if renamed_with_retained_handle { "not_needed" } else { "ok" }
            )
            .unwrap();
        }
    }

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

    /// The mapping behind the cold-start cleanup failure: a delete-pending
    /// entry is still enumerated, and once its last handle closes the name is
    /// gone, so the removal's own opens see ERROR_FILE_NOT_FOUND. That is this
    /// removal's outcome, not a rejection.
    #[test]
    fn checked_tree_removal_completes_when_a_pending_delete_finishes_after_enumeration() {
        let temporary = tempfile::tempdir().unwrap();
        let root_path = temporary.path().join("root");
        let root = Directory::ensure_private(&root_path).unwrap();
        root.create_new(OsStr::new("pending"))
            .unwrap()
            .write_all(b"delete-pending bytes")
            .unwrap();
        root.create_new(OsStr::new("retained"))
            .unwrap()
            .write_all(b"ordinary bytes")
            .unwrap();
        let deletion = open(
            &root_path.join("pending"),
            DELETE | READ_CONTROL | FILE_READ_ATTRIBUTES,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            NameRetention::Movable,
            None,
        )
        .unwrap();
        let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
        // SAFETY: this DELETE-capable handle on the test's own fixture file
        // stays live for the synchronous disposition request, which follows the
        // same class and layout as the checked removal above.
        let status = unsafe {
            SetFileInformationByHandle(
                deletion.as_raw_handle(),
                FileDispositionInfo,
                (&disposition as *const FILE_DISPOSITION_INFO).cast(),
                size_of::<FILE_DISPOSITION_INFO>() as u32,
            )
        };
        assert_ne!(status, 0, "the fixture must reach a delete-pending state");
        // Close the last handle in the window between enumeration and the
        // removal's own open, completing the pending delete right there.
        let mut deletion = Some(deletion);
        let _seam = crate::fs::enumeration_seam::install(move || {
            deletion.take();
        });

        root.remove_tree().unwrap();
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
        #[allow(
            clippy::permissions_set_readonly_false,
            reason = "Windows-only test clears the Windows READONLY attribute before cleanup"
        )]
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
            .unwrap_or_else(|error| panic!("native replacement failed: {error:?}"));
        assert_eq!(
            std::fs::read(directory.path().join("published")).unwrap(),
            b"new"
        );
    }

    struct AccessDiagnostic<'a> {
        protected: u8,
        owner: &'a [u8],
        revision: u8,
        aces: Vec<AceDiagnostic<'a>>,
    }

    struct AceDiagnostic<'a> {
        kind: u8,
        flags: u8,
        size: usize,
        mask_slot: Option<&'a [u8]>,
        payload: &'a [u8],
    }

    // Test-only parser for security::canonical_access_token's current framing.
    // Report component equality, never owner or ACE bytes.
    fn parse_access_token(token: &[u8]) -> AccessDiagnostic<'_> {
        let owner_len = u32::from_le_bytes(token[1..5].try_into().unwrap()) as usize;
        let owner_end = 5 + owner_len;
        let owner = &token[5..owner_end];
        let revision = token[owner_end];
        let ace_count = u32::from_le_bytes(token[owner_end + 1..owner_end + 5].try_into().unwrap());
        let ace_len =
            u32::from_le_bytes(token[owner_end + 5..owner_end + 9].try_into().unwrap()) as usize;
        let ace_bytes = &token[owner_end + 9..];
        assert_eq!(ace_bytes.len(), ace_len);
        let mut remaining = ace_bytes;
        let mut aces = Vec::new();
        for _ in 0..ace_count {
            let size = u16::from_le_bytes(remaining[2..4].try_into().unwrap()) as usize;
            assert!(size >= 4 && size <= remaining.len());
            let (ace, rest) = remaining.split_at(size);
            aces.push(AceDiagnostic {
                kind: ace[0],
                flags: ace[1],
                size,
                mask_slot: (size >= 8).then(|| &ace[4..8]),
                payload: if size >= 8 { &ace[8..] } else { &ace[4..] },
            });
            remaining = rest;
        }
        assert!(remaining.is_empty());
        AccessDiagnostic {
            protected: token[0],
            owner,
            revision,
            aces,
        }
    }

    fn access_token_difference(before: &[u8], after: &[u8]) -> String {
        let before = parse_access_token(before);
        let after = parse_access_token(after);
        let inherited = windows_sys::Win32::Security::INHERITED_ACE as u8;
        let mut inherited_cleared = 0;
        let mut inherited_added = 0;
        let mut other_flag_changes = 0;
        for (left, right) in before.aces.iter().zip(&after.aces) {
            if left.flags & inherited != 0 && right.flags & inherited == 0 {
                inherited_cleared += 1;
            } else if left.flags & inherited == 0 && right.flags & inherited != 0 {
                inherited_added += 1;
            }
            if (left.flags ^ right.flags) & !inherited != 0 {
                other_flag_changes += 1;
            }
        }
        let changed: Vec<_> = before
            .aces
            .iter()
            .zip(&after.aces)
            .enumerate()
            .filter_map(|(index, (left, right))| {
                let same = left.kind == right.kind
                    && left.flags == right.flags
                    && left.size == right.size
                    && left.mask_slot == right.mask_slot
                    && left.payload == right.payload;
                (!same).then(|| {
                    format!(
                        "{index}: type={}, flags={}/0x{:02x}->0x{:02x}/xor=0x{:02x}/inherited_provenance_only={}, size={}, mask_slot={}, payload={}",
                        left.kind == right.kind,
                        left.flags == right.flags,
                        left.flags,
                        right.flags,
                        left.flags ^ right.flags,
                        left.flags ^ right.flags == windows_sys::Win32::Security::INHERITED_ACE as u8,
                        left.size == right.size,
                        left.mask_slot == right.mask_slot,
                        left.payload == right.payload,
                    )
                })
            })
            .take(16)
            .collect();
        format!(
            "protected={}, owner={}, revision={}, ace_count={}/{}, inherited_cleared={}, inherited_added={}, other_flag_changes={}, changed_aces=[{}]",
            before.protected == after.protected,
            before.owner == after.owner,
            before.revision == after.revision,
            before.aces.len(),
            after.aces.len(),
            inherited_cleared,
            inherited_added,
            other_flag_changes,
            changed.join("; "),
        )
    }

    #[test]
    fn access_token_diagnostic_parser_uses_canonical_field_boundaries() {
        let token = [
            1, 2, 0, 0, 0, 0xaa, 0xbb, 2, 2, 0, 0, 0, 12, 0, 0, 0, 0, 0, 8, 0, 1, 2, 3, 4, 0x7f, 0,
            4, 0,
        ];
        let parts = parse_access_token(&token);
        assert_eq!(parts.protected, 1);
        assert_eq!(parts.owner, [0xaa, 0xbb]);
        assert_eq!(parts.revision, 2);
        assert_eq!(parts.aces.len(), 2);
        assert_eq!(parts.aces[0].size, 8);
        assert_eq!(parts.aces[0].mask_slot, Some(&[1, 2, 3, 4][..]));
        assert_eq!(parts.aces[1].mask_slot, None);
        assert!(parts.aces[1].payload.is_empty());
    }

    #[test]
    fn retained_old_access_is_verified_after_checked_posix_replacement() {
        let temporary = tempfile::tempdir().unwrap();
        let directory =
            Directory::open(temporary.path(), Privacy::Inherited, NameRetention::Movable).unwrap();
        directory
            .create_new(OsStr::new("published"))
            .unwrap()
            .write_all(b"old")
            .unwrap();
        let mut old = directory.read(OsStr::new("published")).unwrap();
        let old_identity = info(&old).unwrap().file.identity;
        let stage = directory
            .create_private_directory(OsStr::new("stage"))
            .unwrap();
        let mut candidate = stage.create_new(OsStr::new("payload")).unwrap();
        candidate.write_all(b"new").unwrap();
        let access = crate::fs::copy_file_access(&old, &candidate).unwrap();
        crate::fs::verify_file_access(&old, &access).unwrap();
        let before_publish = file_access_token(&old).unwrap();
        assert!(
            before_publish == file_access_token(&old).unwrap(),
            "source access token changed before publication"
        );
        assert!(
            !retired_access_matches(&old, &before_publish, &before_publish).unwrap(),
            "a still-linked source was treated as a retired file"
        );
        let source =
            Directory::open(stage.path(), Privacy::Inherited, NameRetention::Movable).unwrap();
        assert_eq!(source.identity(), stage.identity());
        directory
            .publish_file_with_access(
                &source,
                OsStr::new("payload"),
                &candidate,
                &access,
                OsStr::new("published"),
                Publication::ReplaceRegular,
            )
            .unwrap();
        let after_publish = file_access_token(&old).unwrap();
        assert!(
            security::same_retired_file_access(&before_publish, &after_publish),
            "retired source changed more than inherited-ACE provenance: {}",
            access_token_difference(&before_publish, &after_publish)
        );
        assert!(
            retired_access_matches(&old, &before_publish, &after_publish).unwrap(),
            "retained old file did not satisfy the checked zero-link retirement boundary"
        );
        crate::fs::finalize_file_access(&old, &candidate).unwrap();
        let after_finalize = file_access_token(&old).unwrap();
        writeln!(
            std::io::stderr().lock(),
            "retained-old access after publish: {}; after finalize: {}",
            access_token_difference(&before_publish, &after_publish),
            access_token_difference(&before_publish, &after_finalize),
        )
        .unwrap();
        crate::fs::verify_retained_file_access(&old, &access).unwrap();
        assert_eq!(crate::fs::retained_file_info(&old).unwrap().links, 0);
        assert_eq!(
            info(&old)
                .err()
                .expect("retired object was accepted")
                .kind(),
            io::ErrorKind::PermissionDenied
        );
        let mut old_bytes = Vec::new();
        old.read_to_end(&mut old_bytes).unwrap();
        assert_eq!(old_bytes, b"old");
        assert_eq!(
            crate::fs::retained_file_info(&old).unwrap().identity,
            old_identity
        );
        assert_eq!(
            std::fs::read(directory.path().join("published")).unwrap(),
            b"new"
        );
    }

    #[test]
    fn retained_access_refuses_a_new_hardlink_before_publication() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = Directory::ensure_private(&temporary.path().join("private")).unwrap();
        directory
            .create_new(OsStr::new("source"))
            .unwrap()
            .write_all(b"source bytes")
            .unwrap();
        let source = directory.read(OsStr::new("source")).unwrap();
        let stage = directory
            .create_private_directory(OsStr::new("stage"))
            .unwrap();
        let candidate = stage.create_new(OsStr::new("payload")).unwrap();
        let access = crate::fs::copy_file_access(&source, &candidate).unwrap();
        crate::fs::verify_retained_file_access(&source, &access).unwrap();
        let unrelated = directory.create_new(OsStr::new("unrelated")).unwrap();
        assert_eq!(
            crate::fs::verify_retained_file_access(&unrelated, &access)
                .unwrap_err()
                .to_string(),
            "access source identity changed during publication"
        );

        let alias = directory.path().join("unexpected-alias");
        std::fs::hard_link(directory.path().join("source"), &alias).unwrap();
        assert_eq!(crate::fs::retained_file_info(&source).unwrap().links, 2);
        let error = crate::fs::verify_retained_file_access(&source, &access).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(
            std::fs::read(directory.path().join("source")).unwrap(),
            b"source bytes"
        );
        assert_eq!(std::fs::read(&alias).unwrap(), b"source bytes");
        assert!(!directory.path().join("published").exists());
    }

    #[test]
    fn replacement_rejects_wrong_parent_authority_before_mutation() {
        let temporary = tempfile::tempdir().unwrap();
        let source_dir = Directory::ensure_private(&temporary.path().join("source")).unwrap();
        let destination_dir =
            Directory::ensure_private(&temporary.path().join("destination")).unwrap();
        let mut candidate = source_dir.create_new(OsStr::new("candidate")).unwrap();
        candidate.write_all(b"new").unwrap();
        let destination = destination_dir.path().join("published");

        let (phase, _) =
            replace_open_destination(&candidate, &candidate, None, &destination).unwrap_err();
        assert_eq!(phase, PublicationPhase::Rejected);

        let wrong_parent = open(
            source_dir.path(),
            FILE_READ_ATTRIBUTES,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            NameRetention::Movable,
            None,
        )
        .unwrap();
        let (phase, _) =
            replace_open_destination(&wrong_parent, &candidate, None, &destination).unwrap_err();
        assert_eq!(phase, PublicationPhase::Rejected);
        assert_eq!(
            std::fs::read(source_dir.path().join("candidate")).unwrap(),
            b"new"
        );
        assert!(!destination.exists());
    }
}
