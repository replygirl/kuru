//! Checked local filesystem operations. These guards retain object identity;
//! they are not a sandbox against another process running as the same user.

use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::path::{Component, Path, PathBuf};

#[cfg(unix)]
#[path = "fs/unix.rs"]
mod native;
#[cfg(windows)]
#[path = "fs/windows.rs"]
mod native;

/// Full native identity, meaningful while its associated handle remains open.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileIdentity {
    volume: u64,
    object: [u8; 16],
}

impl FileIdentity {
    /// Full identity for caller-owned lock keys: little-endian volume followed
    /// by all sixteen opaque object-ID bytes. Only the retained handle makes
    /// this identity authoritative; an old serialized key cannot prove liveness.
    pub fn to_bytes(self) -> [u8; 24] {
        let mut bytes = [0; 24];
        bytes[..8].copy_from_slice(&self.volume.to_le_bytes());
        bytes[8..].copy_from_slice(&self.object);
        bytes
    }
}

/// Information obtained from a regular disk file's actual open handle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileInfo {
    pub identity: FileIdentity,
    pub links: u64,
    pub len: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Privacy {
    /// Ordinary files inherit their containing directory's access policy.
    Inherited,
    /// Only the current process user may receive access grants.
    OwnerOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NameRetention {
    /// Permit legitimate renames while ownership handles remain open.
    Movable,
    /// Windows denies delete sharing; Unix detects substitution on verification.
    Pinned,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Publication {
    New,
    ReplaceRegular,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublicationPhase {
    /// Validation or a definitely unsuccessful native move rejected publication.
    Rejected,
    /// A move may have happened; reconcile identity/receipts before retrying.
    Uncertain,
}

/// A publication error never initiates rollback or removes either object.
#[derive(Debug)]
pub struct PublicationError {
    pub phase: PublicationPhase,
    pub source_identity: Option<FileIdentity>,
    pub destination: PathBuf,
    error: io::Error,
}

impl PublicationError {
    pub fn error(&self) -> &io::Error {
        &self.error
    }
}

impl std::fmt::Display for PublicationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?} publication at {}: {}",
            self.phase,
            self.destination.display(),
            self.error
        )
    }
}

impl std::error::Error for PublicationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

/// Removal consumes the caller's file handle so Windows can finish ordinary
/// deletion. An uncertain result requires reconciliation; it never authorizes
/// deleting a new object which subsequently occupies the same name.
#[derive(Debug)]
pub struct RemovalError {
    pub phase: PublicationPhase,
    pub identity: Option<FileIdentity>,
    pub path: PathBuf,
    error: io::Error,
}

impl std::fmt::Display for RemovalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?} removal at {}: {}",
            self.phase,
            self.path.display(),
            self.error
        )
    }
}

impl std::error::Error for RemovalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

#[derive(Debug)]
struct Anchor {
    path: PathBuf,
    file: File,
    identity: FileIdentity,
}

/// A checked directory and retained handles for its structural ancestors.
#[derive(Debug)]
pub struct Directory {
    anchors: Vec<Anchor>,
    privacy: Privacy,
    retention: NameRetention,
}

#[derive(Clone, Copy)]
enum OpenMode {
    Read,
    ReadWrite,
    New,
    Lock,
}

struct ObjectInfo {
    file: FileInfo,
    directory: bool,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn denied(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

/// Child operations take one literal native component, preserving Unix names.
pub fn validate_component(name: &OsStr) -> io::Result<()> {
    let mut parts = Path::new(name).components();
    if !matches!(parts.next(), Some(Component::Normal(value)) if value == name)
        || parts.next().is_some()
    {
        return Err(invalid("expected one literal filename component"));
    }
    if name.as_encoded_bytes().contains(&0) {
        return Err(invalid("NUL in filename component"));
    }
    #[cfg(windows)]
    {
        let value = name.to_string_lossy();
        if value.contains(['/', '\\', ':']) || value.ends_with(['.', ' ']) {
            return Err(invalid("ambiguous Windows filename component"));
        }
        let base = value
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        if matches!(
            base.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$"
        ) || ["COM", "LPT"].iter().any(|prefix| {
            base.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(
                    suffix,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        }) {
            return Err(invalid("reserved Windows device filename"));
        }
    }
    Ok(())
}

fn component(name: &OsStr) -> io::Result<()> {
    validate_component(name)
}

/// Metadata inspection reports hardlink identity; checked opens reject links.
pub fn regular_file_info(file: &File) -> io::Result<FileInfo> {
    let info = native::info(file)?;
    if info.directory {
        return Err(denied("expected a regular disk file"));
    }
    Ok(info.file)
}

pub fn require_private(file: &File) -> io::Result<()> {
    native::require_private(file)
}

/// Seal a newly written private payload as owner-readable, optionally executable.
pub fn seal_private(file: &File, executable: bool) -> io::Result<()> {
    checked_file(file)?;
    require_private(file)?;
    native::seal_private(file, executable)
}

/// Set ordinary executable access, preserving Windows inherited ACL policy.
pub fn make_executable(file: &File) -> io::Result<()> {
    checked_file(file)?;
    native::make_executable(file)
}

fn checked_file(file: &File) -> io::Result<FileInfo> {
    let info = regular_file_info(file)?;
    if info.links != 1 {
        return Err(denied("regular file must have exactly one hardlink"));
    }
    Ok(info)
}

impl Directory {
    pub fn open(path: &Path, privacy: Privacy, retention: NameRetention) -> io::Result<Self> {
        Self::open_inner(path, privacy, retention, false)
    }

    /// Create missing directories privately, without changing existing access.
    pub fn ensure_private(path: &Path) -> io::Result<Self> {
        Self::open_inner(path, Privacy::OwnerOnly, NameRetention::Movable, true)
    }

    fn open_inner(
        path: &Path,
        privacy: Privacy,
        retention: NameRetention,
        create: bool,
    ) -> io::Result<Self> {
        if !path.is_absolute()
            || path
                .components()
                .any(|part| matches!(part, Component::ParentDir))
        {
            return Err(invalid(
                "checked directory requires an absolute path without parent components",
            ));
        }
        let path = native::normalize(path)?;
        let mut paths: Vec<_> = path.ancestors().map(Path::to_path_buf).collect();
        paths.reverse();
        let mut anchors: Vec<Anchor> = Vec::with_capacity(paths.len());
        for path in paths {
            if let Some(name) = path.file_name() {
                component(name)?;
            }
            let parent = anchors.last().map(|anchor| &anchor.file);
            let file = match native::open_directory(parent, &path, retention) {
                Ok(file) => file,
                Err(error) if create && error.kind() == io::ErrorKind::NotFound => {
                    match native::create_directory(
                        parent.ok_or_else(|| invalid("cannot create a volume root"))?,
                        &path,
                    ) {
                        Ok(()) => (),
                        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
                        Err(error) => return Err(error),
                    }
                    native::open_directory(parent, &path, retention)?
                }
                Err(error) => return Err(error),
            };
            let info = native::info(&file)?;
            if !info.directory {
                return Err(denied("expected a regular directory"));
            }
            anchors.push(Anchor {
                path,
                file,
                identity: info.file.identity,
            });
        }
        let directory = Self {
            anchors,
            privacy,
            retention,
        };
        if privacy == Privacy::OwnerOnly {
            directory.check_private()?;
        }
        Ok(directory)
    }

    fn anchor(&self) -> &Anchor {
        self.anchors.last().expect("an absolute path has a root")
    }

    pub fn path(&self) -> &Path {
        &self.anchor().path
    }

    pub fn identity(&self) -> FileIdentity {
        self.anchor().identity
    }

    fn check_private(&self) -> io::Result<()> {
        let files: Vec<_> = self.anchors.iter().map(|anchor| &anchor.file).collect();
        native::private_chain(&files)
    }

    fn revalidate(&self) -> io::Result<()> {
        let mut parent = None;
        for held in &self.anchors {
            let current = native::open_directory(parent.as_ref(), &held.path, self.retention)?;
            let info = native::info(&current)?;
            if !info.directory || info.file.identity != held.identity {
                return Err(denied("directory name or ancestor identity changed"));
            }
            parent = Some(current);
        }
        if self.privacy == Privacy::OwnerOnly {
            self.check_private()?;
        }
        Ok(())
    }

    /// Whether this directory is the held ancestor or lies beneath it, using
    /// full native identities rather than case-sensitive pathname prefixes.
    /// Both retained directory paths must still identify their original objects.
    pub fn is_within(&self, ancestor: &Directory) -> io::Result<bool> {
        self.revalidate()?;
        ancestor.revalidate()?;
        Ok(self
            .anchors
            .iter()
            .any(|anchor| anchor.identity == ancestor.identity()))
    }

    /// Exclusively create a protected private child, even under an ordinary
    /// installation directory. Existing names are never adopted or repaired.
    pub fn create_private_directory(&self, name: &OsStr) -> io::Result<Directory> {
        component(name)?;
        self.revalidate()?;
        let path = self.path().join(name);
        native::create_directory(&self.anchor().file, &path)?;
        self.revalidate()?;
        Self::open(&path, Privacy::OwnerOnly, NameRetention::Movable)
    }

    fn open_file(&self, name: &OsStr, mode: OpenMode) -> io::Result<File> {
        component(name)?;
        self.revalidate()?;
        let privacy = if matches!(mode, OpenMode::Lock) {
            Privacy::OwnerOnly
        } else {
            self.privacy
        };
        if privacy == Privacy::OwnerOnly {
            self.check_private()?;
        }
        let file = native::open_file(
            &self.anchor().file,
            &self.path().join(name),
            mode,
            privacy,
            self.retention,
        )?;
        checked_file(&file)?;
        if privacy == Privacy::OwnerOnly {
            require_private(&file)?;
        }
        self.revalidate()?;
        Ok(file)
    }

    pub fn read(&self, name: &OsStr) -> io::Result<File> {
        self.open_file(name, OpenMode::Read)
    }

    /// Open an existing checked file without truncation.
    pub fn read_write(&self, name: &OsStr) -> io::Result<File> {
        self.open_file(name, OpenMode::ReadWrite)
    }

    pub fn create_new(&self, name: &OsStr) -> io::Result<File> {
        self.open_file(name, OpenMode::New)
    }

    /// Opens a stable private lock object; callers own locking and bounded waits.
    pub fn lock_file(&self, name: &OsStr) -> io::Result<File> {
        self.open_file(name, OpenMode::Lock)
    }

    /// Call after locking and before using an object whose name must be stable.
    pub fn verify(&self, name: &OsStr, file: &File) -> io::Result<()> {
        let held = checked_file(file)?;
        let current = self.read(name)?;
        if held.identity != checked_file(&current)?.identity {
            return Err(denied("file name no longer identifies the held object"));
        }
        Ok(())
    }

    /// Remove exactly one checked regular file and consume its held handle.
    /// Success means namespace disappearance was observed after closing our
    /// handles. Other live handles or mapped images can leave removal uncertain.
    /// This does not change ACLs/attributes, recursively delete, or retry.
    pub fn remove_file(&self, name: &OsStr, file: File) -> Result<(), RemovalError> {
        self.remove_file_then(name, file, || Ok(()))
    }

    fn remove_file_then(
        &self,
        name: &OsStr,
        file: File,
        after_remove: impl FnOnce() -> io::Result<()>,
    ) -> Result<(), RemovalError> {
        let identity = checked_file(&file).ok().map(|info| info.identity);
        let path = self.path().join(name);
        let failure = |phase, error| RemovalError {
            phase,
            identity,
            path: path.clone(),
            error,
        };
        self.verify(name, &file)
            .map_err(|error| failure(PublicationPhase::Rejected, error))?;
        native::remove(&self.anchor().file, &path, file)
            .map_err(|(phase, error)| failure(phase, error))?;
        after_remove().map_err(|error| failure(PublicationPhase::Uncertain, error))?;
        self.revalidate()
            .map_err(|error| failure(PublicationPhase::Uncertain, error))?;
        match self.read(name) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(failure(PublicationPhase::Uncertain, error)),
            Ok(_) => Err(failure(
                PublicationPhase::Uncertain,
                denied("removed name is still occupied"),
            )),
        }
    }

    pub fn publish_file(
        &self,
        source: &Directory,
        name: &OsStr,
        file: &File,
        destination: &OsStr,
        policy: Publication,
    ) -> Result<(), PublicationError> {
        self.transfer_file_then((source, name, file), destination, policy, true, || Ok(()))
    }

    /// Rename an unchanged existing file without flushing a read-only source.
    /// This retains the same validation and native write-through move contract
    /// as publication, but cannot establish durability for unwritten payloads.
    /// Newly written candidates and receipts must use `publish_file` instead.
    pub fn rename_file(
        &self,
        source: &Directory,
        name: &OsStr,
        file: &File,
        destination: &OsStr,
        policy: Publication,
    ) -> Result<(), PublicationError> {
        self.transfer_file_then((source, name, file), destination, policy, false, || Ok(()))
    }

    fn transfer_file_then(
        &self,
        (source, name, file): (&Directory, &OsStr, &File),
        destination: &OsStr,
        policy: Publication,
        flush_payload: bool,
        after_move: impl FnOnce() -> io::Result<()>,
    ) -> Result<(), PublicationError> {
        let identity = checked_file(file).ok().map(|info| info.identity);
        let target = self.path().join(destination);
        let failure = |phase, error| PublicationError {
            phase,
            source_identity: identity,
            destination: target.clone(),
            error,
        };
        let preflight = || -> io::Result<()> {
            component(destination)?;
            source.verify(name, file)?;
            if self.privacy == Privacy::OwnerOnly {
                require_private(file)?;
            }
            self.revalidate()?;
            if source.identity().volume != self.identity().volume {
                return Err(invalid("publication must stay on the same volume"));
            }
            match self.read(destination) {
                Ok(existing) => {
                    if policy == Publication::New {
                        return Err(io::Error::new(
                            io::ErrorKind::AlreadyExists,
                            "publication target already exists",
                        ));
                    }
                    if Some(regular_file_info(&existing)?.identity) == identity {
                        return Err(invalid(
                            "publication source and destination are the same file",
                        ));
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            if flush_payload {
                file.sync_all()?;
            }
            Ok(())
        };
        preflight().map_err(|error| failure(PublicationPhase::Rejected, error))?;
        native::publish(
            &source.anchor().file,
            &source.path().join(name),
            &self.anchor().file,
            &target,
            policy,
        )
        .map_err(|(phase, error)| failure(phase, error))?;
        after_move().map_err(|error| failure(PublicationPhase::Uncertain, error))?;
        self.verify(destination, file)
            .map_err(|error| failure(PublicationPhase::Uncertain, error))
    }

    /// Move a stopped directory to an absent name. The caller retains `source`
    /// across any uncertain result and owns process quiescence separately.
    /// Source and destination must have the same privacy policy: moving a
    /// directory does not convert its access policy or that of its descendants.
    pub fn move_new_directory(
        &self,
        source: &Directory,
        destination: &OsStr,
    ) -> Result<Directory, PublicationError> {
        let target = self.path().join(destination);
        let failure = |phase, error| PublicationError {
            phase,
            source_identity: Some(source.identity()),
            destination: target.clone(),
            error,
        };
        let preflight = || -> io::Result<()> {
            component(destination)?;
            if source.privacy != self.privacy {
                return Err(invalid(
                    "directory publication cannot change privacy policy",
                ));
            }
            source.revalidate()?;
            self.revalidate()?;
            if source.identity().volume != self.identity().volume {
                return Err(invalid(
                    "directory publication must stay on the same volume",
                ));
            }
            if source.anchors.len() < 2 {
                return Err(invalid("cannot publish a filesystem root"));
            }
            match std::fs::symlink_metadata(&target) {
                Ok(_) => Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "directory publication target exists",
                )),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error),
            }
        };
        preflight().map_err(|error| failure(PublicationPhase::Rejected, error))?;
        let parent = &source.anchors[source.anchors.len() - 2].file;
        native::publish(
            parent,
            source.path(),
            &self.anchor().file,
            &target,
            Publication::New,
        )
        .map_err(|(phase, error)| failure(phase, error))?;
        let result = Self::open(&target, source.privacy, source.retention)
            .map_err(|error| failure(PublicationPhase::Uncertain, error))?;
        if result.identity() != source.identity() {
            return Err(failure(
                PublicationPhase::Uncertain,
                denied("published directory identity changed"),
            ));
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn removal_reconciles_completion_failures_without_touching_new_occupants() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = Directory::ensure_private(&temporary.path().join("private")).unwrap();
        let name = OsStr::new("retired");
        let file = directory.create_new(name).unwrap();
        let error = directory
            .remove_file_then(name, file, || Err(io::Error::other("completion failed")))
            .unwrap_err();
        assert_eq!(error.phase, PublicationPhase::Uncertain);
        assert!(!directory.path().join(name).exists());
        let file = directory.create_new(name).unwrap();
        let error = directory
            .remove_file_then(name, file, || {
                directory.create_new(name)?.write_all(b"new occupant")
            })
            .unwrap_err();
        assert_eq!(error.phase, PublicationPhase::Uncertain);
        assert_eq!(
            std::fs::read(directory.path().join(name)).unwrap(),
            b"new occupant"
        );
    }

    #[test]
    fn a_real_move_retains_reconciliation_evidence_when_completion_fails() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = Directory::ensure_private(&temporary.path().join("private")).unwrap();
        let mut candidate = directory.create_new(OsStr::new("candidate")).unwrap();
        candidate.write_all(b"new committed bytes").unwrap();
        let identity = regular_file_info(&candidate).unwrap().identity;
        let error = directory
            .transfer_file_then(
                (&directory, OsStr::new("candidate"), &candidate),
                OsStr::new("published"),
                Publication::New,
                true,
                || {
                    Err(io::Error::other(
                        "controlled completion failure after actual native move",
                    ))
                },
            )
            .unwrap_err();
        assert_eq!(error.phase, PublicationPhase::Uncertain);
        assert_eq!(error.source_identity, Some(identity));
        assert_eq!(
            regular_file_info(&directory.read(OsStr::new("published")).unwrap())
                .unwrap()
                .identity,
            identity
        );
        assert!(!directory.path().join("candidate").exists());
        assert_eq!(
            std::fs::read(&error.destination).unwrap(),
            b"new committed bytes"
        );
        assert!(error.to_string().contains("controlled completion failure"));
        assert!(std::error::Error::source(&error).is_some());
    }
}
