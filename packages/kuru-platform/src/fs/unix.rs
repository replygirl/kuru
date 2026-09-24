use super::*;

pub(super) fn file_access_token(source: &File) -> io::Result<Vec<u8>> {
    let metadata = source.metadata()?;
    let mut token = Vec::with_capacity(12);
    token.extend_from_slice(&metadata.uid().to_le_bytes());
    token.extend_from_slice(&metadata.gid().to_le_bytes());
    token.extend_from_slice(&metadata.mode().to_le_bytes());
    Ok(token)
}

pub(super) fn copy_file_access(source: &File, staged: &File) -> io::Result<()> {
    let source_info = source.metadata()?;
    let staged_info = staged.metadata()?;
    if source_info.uid() != staged_info.uid() || source_info.gid() != staged_info.gid() {
        return Err(denied(
            "staged file cannot preserve source owner and group access",
        ));
    }
    staged.set_permissions(source_info.permissions())
}

pub(super) fn prepare_file_replacement(_: &File) -> io::Result<Option<File>> {
    Ok(None)
}

pub(super) fn finalize_file_access(_: &File, _: &File) -> io::Result<()> {
    Ok(())
}
use rustix::fs::{
    AtFlags, Dir, Mode, OFlags, RenameFlags, mkdirat, openat, renameat, renameat_with, unlinkat,
};
use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};

pub(super) fn normalize(path: &Path) -> io::Result<PathBuf> {
    // macOS exposes these OS-owned aliases to tempfile and ordinary callers.
    // Recognize only the exact system mapping, never arbitrary user symlinks.
    #[cfg(target_os = "macos")]
    for name in ["tmp", "var", "etc"] {
        let alias = Path::new("/").join(name);
        if let Ok(suffix) = path.strip_prefix(&alias) {
            let metadata = std::fs::symlink_metadata(&alias)?;
            let actual = Path::new("/private").join(name);
            if metadata.file_type().is_symlink()
                && metadata.uid() == 0
                && Path::new("/").join(std::fs::read_link(&alias)?) == actual
            {
                return Ok(actual.join(suffix));
            }
        }
    }
    Ok(path.to_path_buf())
}

pub(super) fn info(file: &File) -> io::Result<ObjectInfo> {
    let metadata = file.metadata()?;
    if !metadata.is_file() && !metadata.is_dir() {
        return Err(denied("expected a regular file or directory"));
    }
    let mut object = [0; 16];
    object[..8].copy_from_slice(&metadata.ino().to_le_bytes());
    Ok(ObjectInfo {
        file: FileInfo {
            identity: FileIdentity {
                volume: metadata.dev(),
                object,
            },
            links: metadata.nlink(),
            len: metadata.len(),
        },
        directory: metadata.is_dir(),
    })
}

pub(super) fn retained_info(file: &File) -> io::Result<ObjectInfo> {
    info(file)
}

pub(super) fn require_private(file: &File) -> io::Result<()> {
    let metadata = file.metadata()?;
    if metadata.uid() != rustix::process::geteuid().as_raw() || metadata.mode() & 0o077 != 0 {
        return Err(denied(
            "private object must belong to the current user with no group or other permissions",
        ));
    }
    Ok(())
}

pub(super) fn private_chain(files: &[&File]) -> io::Result<()> {
    require_private(
        files
            .last()
            .ok_or_else(|| invalid("missing private root"))?,
    )
}

pub(super) fn open_directory(
    parent: Option<&File>,
    path: &Path,
    _: NameRetention,
) -> io::Result<File> {
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let fd = match parent {
        Some(parent) => openat(
            parent,
            path.file_name()
                .ok_or_else(|| invalid("missing directory name"))?,
            flags,
            Mode::empty(),
        ),
        None => openat(rustix::fs::CWD, path, flags, Mode::empty()),
    }?;
    Ok(File::from(fd))
}

pub(super) fn create_directory(parent: &File, path: &Path) -> io::Result<()> {
    mkdirat(
        parent,
        path.file_name()
            .ok_or_else(|| invalid("missing directory name"))?,
        Mode::from_raw_mode(0o700),
    )?;
    Ok(())
}

pub(super) fn open_file(
    parent: &File,
    path: &Path,
    mode: OpenMode,
    privacy: Privacy,
    _: NameRetention,
) -> io::Result<File> {
    let mut flags = OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
    flags |= match mode {
        OpenMode::Read => OFlags::RDONLY,
        OpenMode::ReadWrite => OFlags::RDWR,
        OpenMode::New => OFlags::RDWR | OFlags::CREATE | OFlags::EXCL,
        OpenMode::Lock => OFlags::RDWR | OFlags::CREATE | OFlags::EXCL,
    };
    let permissions = if privacy == Privacy::OwnerOnly {
        0o600
    } else {
        0o666
    };
    let name = path
        .file_name()
        .ok_or_else(|| invalid("missing filename"))?;
    let opened = openat(parent, name, flags, Mode::from_raw_mode(permissions));
    let file = match opened {
        Err(rustix::io::Errno::EXIST) if matches!(mode, OpenMode::Lock) => {
            // Distinguish our exclusive creation from opening the stable object
            // another owner created. Never recreate an object removed in between.
            openat(
                parent,
                name,
                flags & !(OFlags::CREATE | OFlags::EXCL),
                Mode::empty(),
            )?
        }
        result => result?,
    };
    Ok(File::from(file))
}

pub(super) fn seal_private(file: &File, executable: bool) -> io::Result<()> {
    rustix::fs::fchmod(
        file,
        Mode::from_raw_mode(if executable { 0o500 } else { 0o400 }),
    )?;
    Ok(())
}

pub(super) fn make_executable(file: &File) -> io::Result<()> {
    rustix::fs::fchmod(file, Mode::from_raw_mode(0o755))?;
    Ok(())
}

pub(super) fn remove(
    parent: &File,
    path: &Path,
    held: File,
) -> Result<(), (PublicationPhase, io::Error)> {
    rustix::fs::unlinkat(
        parent,
        path.file_name().unwrap(),
        rustix::fs::AtFlags::empty(),
    )
    .map_err(|error| (PublicationPhase::Rejected, error.into()))?;
    drop(held);
    parent
        .sync_all()
        .map_err(|error| (PublicationPhase::Uncertain, error))
}

const MAX_TREE_DEPTH: usize = 128;

pub(super) fn remove_tree(
    parent: &File,
    _: &[(&Path, FileIdentity)],
    _: &Path,
    name: &std::ffi::OsStr,
    held: File,
    expected: FileIdentity,
) -> Result<(), (PublicationPhase, io::Error)> {
    let mut removed = false;
    // A root that is already absent is the outcome this removal wants, not a
    // rejection: an earlier attempt, or the holder of a pending delete, got
    // there first. Only absence is accepted here; every other failure stands.
    match verify_named(parent, name, expected, &mut removed) {
        Ok(()) => {}
        Err((_, error)) if error.kind() == io::ErrorKind::NotFound => {
            drop(held);
            verify_absent(parent, name, &mut removed)?;
            return parent
                .sync_all()
                .map_err(|error| (PublicationPhase::Uncertain, error));
        }
        Err(failure) => return Err(failure),
    }
    remove_children(&held, &mut removed, 0)?;
    verify_named(parent, name, expected, &mut removed)?;
    unlinkat(parent, name, AtFlags::REMOVEDIR).map_err(|error| (phase(removed), error.into()))?;
    drop(held);
    verify_absent(parent, name, &mut removed)?;
    parent
        .sync_all()
        .map_err(|error| (PublicationPhase::Uncertain, error))
}

fn phase(removed: bool) -> PublicationPhase {
    if removed {
        PublicationPhase::Uncertain
    } else {
        PublicationPhase::Rejected
    }
}

fn verify_named(
    parent: &File,
    name: &std::ffi::OsStr,
    expected: FileIdentity,
    removed: &mut bool,
) -> Result<(), (PublicationPhase, io::Error)> {
    let current = openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|error| (phase(*removed), error.into()))?;
    let current = File::from(current);
    let actual = info(&current).map_err(|error| (phase(*removed), error))?;
    if actual.file.identity != expected {
        return Err((
            phase(*removed),
            denied("directory entry no longer identifies the held object"),
        ));
    }
    Ok(())
}

fn verify_absent(
    parent: &File,
    name: &std::ffi::OsStr,
    _: &mut bool,
) -> Result<(), (PublicationPhase, io::Error)> {
    match openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Err(error) if error == rustix::io::Errno::NOENT => Ok(()),
        Err(error) => Err((PublicationPhase::Uncertain, error.into())),
        Ok(_) => Err((
            PublicationPhase::Uncertain,
            denied("removed directory name is still occupied"),
        )),
    }
}

fn remove_children(
    directory: &File,
    removed: &mut bool,
    depth: usize,
) -> Result<(), (PublicationPhase, io::Error)> {
    if depth >= MAX_TREE_DEPTH {
        return Err((
            phase(*removed),
            invalid("checked tree removal depth exceeded"),
        ));
    }
    let mut entries = Dir::read_from(directory).map_err(|error| (phase(*removed), error.into()))?;
    while let Some(entry) = entries.read() {
        let entry = entry.map_err(|error| (phase(*removed), error.into()))?;
        let name = std::ffi::OsStr::from_bytes(entry.file_name().to_bytes());
        if matches!(name.as_bytes(), b"." | b"..") {
            continue;
        }
        #[cfg(test)]
        super::enumeration_seam::observe();
        let child = openat(
            directory,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        );
        match child {
            Ok(child) => {
                let child = File::from(child);
                let info = info(&child).map_err(|error| (phase(*removed), error))?;
                if !info.directory {
                    return Err((phase(*removed), denied("expected a regular directory")));
                }
                remove_children(&child, removed, depth + 1)?;
                verify_named(directory, name, info.file.identity, removed)?;
                unlinkat(directory, name, AtFlags::REMOVEDIR)
                    .map_err(|error| (phase(*removed), error.into()))?;
                drop(child);
                *removed = true;
            }
            Err(error) if error == rustix::io::Errno::NOTDIR => {
                let child = match openat(
                    directory,
                    name,
                    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                    Mode::empty(),
                ) {
                    Ok(child) => child,
                    // The enumerated name disappeared between the two opens.
                    Err(error) if error == rustix::io::Errno::NOENT => continue,
                    Err(error) => return Err((phase(*removed), error.into())),
                };
                let child = File::from(child);
                let info = info(&child).map_err(|error| (phase(*removed), error))?;
                if info.directory {
                    return Err((phase(*removed), denied("expected a regular file")));
                }
                verify_named(directory, name, info.file.identity, removed)?;
                unlinkat(directory, name, AtFlags::empty())
                    .map_err(|error| (phase(*removed), error.into()))?;
                drop(child);
                *removed = true;
            }
            // An enumerated name already gone by the time this removal reaches
            // it is the outcome this loop wants. `removed` stays untouched: we
            // performed no removal, and the final absence check still proves
            // the tree's disappearance.
            Err(error) if error == rustix::io::Errno::NOENT => continue,
            Err(error) => return Err((phase(*removed), error.into())),
        }
    }
    Ok(())
}

pub(super) fn publish(
    source_parent: &File,
    source: &Path,
    _: &File,
    _: Option<&File>,
    destination_parent: &File,
    destination: &Path,
    policy: Publication,
) -> Result<(), (PublicationPhase, io::Error)> {
    let moved = match policy {
        Publication::New => renameat_with(
            source_parent,
            source.file_name().unwrap(),
            destination_parent,
            destination.file_name().unwrap(),
            RenameFlags::NOREPLACE,
        ),
        Publication::ReplaceRegular => renameat(
            source_parent,
            source.file_name().unwrap(),
            destination_parent,
            destination.file_name().unwrap(),
        ),
    };
    moved.map_err(|error| (PublicationPhase::Rejected, error.into()))?;
    source_parent
        .sync_all()
        .and_then(|()| destination_parent.sync_all())
        .map_err(|error| (PublicationPhase::Uncertain, error))
}
