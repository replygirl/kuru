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

/// Test-only seam inside checked tree removal, invoked on every enumerated
/// entry after the enumeration observes its name and before any handle on that
/// name is opened. It exists so a test can make an enumerated entry disappear
/// in exactly the window a completing pending delete uses on Windows.
#[cfg(test)]
pub(crate) mod enumeration_seam {
    use std::cell::RefCell;

    thread_local! {
        static OBSERVER: RefCell<Option<Box<dyn FnMut()>>> = const { RefCell::new(None) };
    }

    /// Removes the installed observer, keeping the seam local to one test.
    pub(crate) struct Installed;

    impl Drop for Installed {
        fn drop(&mut self) {
            OBSERVER.with(|slot| slot.borrow_mut().take());
        }
    }

    pub(crate) fn install(observer: impl FnMut() + 'static) -> Installed {
        OBSERVER.with(|slot| *slot.borrow_mut() = Some(Box::new(observer)));
        Installed
    }

    pub(crate) fn observe() {
        OBSERVER.with(|slot| {
            if let Ok(mut slot) = slot.try_borrow_mut()
                && let Some(observer) = slot.as_mut()
            {
                observer();
            }
        });
    }
}

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

/// Opaque access-policy capture from one retained regular-file handle.
pub struct FileAccessToken {
    bytes: Vec<u8>,
    staged_identity: FileIdentity,
    replacement: Option<File>,
}

/// Copy the access policy of a checked regular file onto a newly staged file.
/// Callers must keep the staged file behind a private directory until publish:
/// this operation can intentionally grant ordinary project access.
pub fn copy_file_access(source: &File, staged: &File) -> io::Result<FileAccessToken> {
    checked_file(source)?;
    let staged_identity = checked_file(staged)?.identity;
    let bytes = native::file_access_token(source)?;
    // Retain the exact private stage's replacement authority before copying an
    // ordinary target DACL that may deliberately omit DELETE on the file.
    let replacement = native::prepare_file_replacement(staged)?;
    native::copy_file_access(source, staged)?;
    let token = FileAccessToken {
        bytes,
        staged_identity,
        replacement,
    };
    verify_file_access(source, &token)?;
    Ok(token)
}

/// Detect a source access-policy change on the same retained handle. A
/// replaced source may have zero links after publication; any new alias is
/// still refused. This does not lock out an external ACL writer.
pub fn verify_file_access(source: &File, expected: &FileAccessToken) -> io::Result<()> {
    if regular_file_info(source)?.links > 1 {
        return Err(denied("access source gained another hardlink"));
    }
    if native::file_access_token(source)? != expected.bytes {
        return Err(denied("file access policy changed during publication"));
    }
    Ok(())
}

/// After a checked move into the destination parent, restore the source's
/// inheritance behavior through the still-retained published file handle.
/// A caller must not settle its effect as applied until this succeeds.
pub fn finalize_file_access(source: &File, published: &File) -> io::Result<()> {
    // A replaced source can have zero links after the checked publication,
    // while its retained handle still carries the access policy we copied.
    // It was checked before publication; reject any newly linked alias.
    if regular_file_info(source)?.links > 1 {
        return Err(denied("access source gained another hardlink"));
    }
    checked_file(published)?;
    native::finalize_file_access(source, published)
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
    operation: &'static str,
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
            "{:?} publication during {} at {}: {}",
            self.phase,
            self.operation,
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
///
/// Absence is the intended outcome of a removal, never a rejection: a target
/// that is already gone when a checked removal reaches it - an enumerated entry
/// whose pending delete completed, or a root removed by an earlier attempt -
/// completes that removal. Denied and uncertain results keep their meaning, and
/// `path` always names the tree root, never the descendant that failed.
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

fn not_found(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, message)
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
    if info.links == 0 {
        return Err(not_found("regular file was unlinked"));
    }
    if info.links > 1 {
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

    /// Reopen each retained pathname and verify that it still resolves to the
    /// native objects held by this capability.
    ///
    /// This detects a replacement observed at this check. It does not bind a
    /// later pathname-based child working directory atomically to the retained
    /// directory, and it is not a sandbox or authorization decision.
    pub fn revalidate(&self) -> io::Result<()> {
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

    /// Remove this checked directory and every checked descendant. This
    /// consumes the held root so native Windows deletion can complete. The
    /// caller owns inventory and retry policy; a possible partial deletion is
    /// reported as uncertain and never authorizes selecting a new pathname.
    pub fn remove_tree(self) -> Result<(), RemovalError> {
        let identity = self.identity();
        let path = self.path().to_path_buf();
        if self.anchors.len() < 2 {
            return Err(RemovalError {
                phase: PublicationPhase::Rejected,
                identity: Some(identity),
                path,
                error: invalid("cannot remove a filesystem root"),
            });
        }
        let name = path.file_name().ok_or_else(|| RemovalError {
            phase: PublicationPhase::Rejected,
            identity: Some(identity),
            path: path.clone(),
            error: invalid("missing directory name"),
        })?;
        component(name).map_err(|error| RemovalError {
            phase: PublicationPhase::Rejected,
            identity: Some(identity),
            path: path.clone(),
            error,
        })?;
        if self.privacy != Privacy::OwnerOnly {
            return Err(RemovalError {
                phase: PublicationPhase::Rejected,
                identity: Some(identity),
                path,
                error: denied("checked tree removal requires an owner-private root"),
            });
        }
        if self.retention != NameRetention::Movable {
            return Err(RemovalError {
                phase: PublicationPhase::Rejected,
                identity: Some(identity),
                path,
                error: denied("checked tree removal requires a movable root handle"),
            });
        }
        self.revalidate().map_err(|error| RemovalError {
            phase: PublicationPhase::Rejected,
            identity: Some(identity),
            path: path.clone(),
            error,
        })?;
        let retention = self.retention;
        let mut anchors = self.anchors;
        let root = anchors.pop().expect("root count checked");
        let parent = anchors
            .last()
            .expect("root count checked")
            .file
            .try_clone()
            .map_err(|error| RemovalError {
                phase: PublicationPhase::Rejected,
                identity: Some(identity),
                path: path.clone(),
                error,
            })?;
        let ancestor_paths: Vec<_> = anchors
            .iter()
            .map(|anchor| (anchor.path.as_path(), anchor.identity))
            .collect();
        native::remove_tree(&parent, &ancestor_paths, &path, name, root.file, identity).map_err(
            |(phase, error)| RemovalError {
                phase,
                identity: Some(identity),
                path: path.clone(),
                error,
            },
        )?;
        Self::revalidate_ancestors(&anchors, retention).map_err(|error| RemovalError {
            phase: PublicationPhase::Uncertain,
            identity: Some(identity),
            path,
            error,
        })
    }

    fn revalidate_ancestors(anchors: &[Anchor], retention: NameRetention) -> io::Result<()> {
        let mut parent = None;
        for held in anchors {
            let current = native::open_directory(parent.as_ref(), &held.path, retention)?;
            let info = native::info(&current)?;
            if !info.directory || info.file.identity != held.identity {
                return Err(denied("directory ancestor identity changed during removal"));
            }
            parent = Some(current);
        }
        Ok(())
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
        self.transfer_file_then(
            (source, name, file),
            destination,
            policy,
            true,
            None,
            || Ok(()),
        )
    }

    /// Publish a staged file whose ordinary access policy was copied through
    /// [`copy_file_access`]. The opaque token retains the exact private stage's
    /// narrow Windows replacement authority across that DACL transition.
    pub fn publish_file_with_access(
        &self,
        source: &Directory,
        name: &OsStr,
        file: &File,
        access: &FileAccessToken,
        destination: &OsStr,
        policy: Publication,
    ) -> Result<(), PublicationError> {
        self.transfer_file_then(
            (source, name, file),
            destination,
            policy,
            true,
            Some(access),
            || Ok(()),
        )
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
        self.transfer_file_then(
            (source, name, file),
            destination,
            policy,
            false,
            None,
            || Ok(()),
        )
    }

    fn transfer_file_then(
        &self,
        (source, name, file): (&Directory, &OsStr, &File),
        destination: &OsStr,
        policy: Publication,
        flush_payload: bool,
        access: Option<&FileAccessToken>,
        after_move: impl FnOnce() -> io::Result<()>,
    ) -> Result<(), PublicationError> {
        let identity = checked_file(file).ok().map(|info| info.identity);
        let target = self.path().join(destination);
        let failure = |phase, operation, error| PublicationError {
            phase,
            source_identity: identity,
            destination: target.clone(),
            operation,
            error,
        };
        let preflight = || -> io::Result<()> {
            component(destination)?;
            source.verify(name, file)?;
            if let Some(access) = access
                && access.staged_identity != checked_file(file)?.identity
            {
                return Err(denied("copied access token belongs to another staged file"));
            }
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
        preflight().map_err(|error| failure(PublicationPhase::Rejected, "preflight", error))?;
        native::publish(
            &source.anchor().file,
            &source.path().join(name),
            file,
            access.and_then(|access| access.replacement.as_ref()),
            &self.anchor().file,
            &target,
            policy,
        )
        .map_err(|(phase, error)| failure(phase, "native-move", error))?;
        after_move()
            .map_err(|error| failure(PublicationPhase::Uncertain, "postmove-completion", error))?;
        self.verify(destination, file)
            .map_err(|error| failure(PublicationPhase::Uncertain, "identity-verification", error))
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
        let failure = |phase, operation, error| PublicationError {
            phase,
            source_identity: Some(source.identity()),
            destination: target.clone(),
            operation,
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
        preflight().map_err(|error| failure(PublicationPhase::Rejected, "preflight", error))?;
        let parent = &source.anchors[source.anchors.len() - 2].file;
        native::publish(
            parent,
            source.path(),
            &source.anchor().file,
            None,
            &self.anchor().file,
            &target,
            Publication::New,
        )
        .map_err(|(phase, error)| failure(phase, "native-move", error))?;
        let result = Self::open(&target, source.privacy, source.retention)
            .map_err(|error| failure(PublicationPhase::Uncertain, "postmove-open", error))?;
        if result.identity() != source.identity() {
            return Err(failure(
                PublicationPhase::Uncertain,
                "identity-verification",
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

    #[cfg(unix)]
    #[test]
    fn staged_access_token_rejects_a_held_source_mode_change() {
        use std::os::unix::fs::PermissionsExt;

        let temporary = tempfile::tempdir().unwrap();
        let parent =
            Directory::open(temporary.path(), Privacy::Inherited, NameRetention::Movable).unwrap();
        let source = parent.create_new(OsStr::new("source")).unwrap();
        source
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .unwrap();
        let stage = parent
            .create_private_directory(OsStr::new("stage"))
            .unwrap();
        let candidate = stage.create_new(OsStr::new("payload")).unwrap();
        let token = copy_file_access(&source, &candidate).unwrap();
        source
            .set_permissions(std::fs::Permissions::from_mode(0o400))
            .unwrap();
        assert!(verify_file_access(&source, &token).is_err());
        assert_eq!(
            std::fs::metadata(stage.path())
                .unwrap()
                .permissions()
                .mode()
                & 0o077,
            0,
            "staged payload directory lost owner-only traversal"
        );
    }

    #[test]
    fn checked_tree_removal_consumes_only_regular_private_descendants() {
        let temporary = tempfile::tempdir().unwrap();
        let root = Directory::ensure_private(&temporary.path().join("root")).unwrap();
        let nested = Directory::ensure_private(&root.path().join("nested")).unwrap();
        nested
            .create_new(OsStr::new("record"))
            .unwrap()
            .write_all(b"private bytes")
            .unwrap();
        // The descendant's checked handle would legitimately prevent native
        // Windows deletion, so close it before observing a successful tree
        // removal on every supported host.
        drop(nested);
        let outside = Directory::ensure_private(&temporary.path().join("outside")).unwrap();
        outside
            .create_new(OsStr::new("sentinel"))
            .unwrap()
            .write_all(b"outside bytes")
            .unwrap();
        drop(outside);

        root.remove_tree().unwrap();
        assert!(!temporary.path().join("root").exists());
        assert_eq!(
            std::fs::read(temporary.path().join("outside/sentinel")).unwrap(),
            b"outside bytes"
        );
    }

    #[test]
    fn checked_tree_removal_completes_when_an_enumerated_child_vanishes_first() {
        let temporary = tempfile::tempdir().unwrap();
        let root = Directory::ensure_private(&temporary.path().join("root")).unwrap();
        for name in ["first", "second"] {
            root.create_new(OsStr::new(name))
                .unwrap()
                .write_all(b"private bytes")
                .unwrap();
        }
        let outside = Directory::ensure_private(&temporary.path().join("outside")).unwrap();
        outside
            .create_new(OsStr::new("sentinel"))
            .unwrap()
            .write_all(b"outside bytes")
            .unwrap();
        drop(outside);
        // Delete an enumerated entry in the window between enumeration and the
        // handles this removal opens on it, the window a completing Windows
        // pending delete uses.
        let vanishing = temporary.path().join("root/first");
        let _seam = enumeration_seam::install(move || {
            let _ = std::fs::remove_file(&vanishing);
        });

        root.remove_tree().unwrap();
        assert!(!temporary.path().join("root").exists());
        assert_eq!(
            std::fs::read(temporary.path().join("outside/sentinel")).unwrap(),
            b"outside bytes"
        );
    }

    #[cfg(unix)]
    #[test]
    fn checked_tree_removal_rejects_a_symlink_descendant() {
        use std::os::unix::fs::symlink;

        let temporary = tempfile::tempdir().unwrap();
        let root = Directory::ensure_private(&temporary.path().join("root")).unwrap();
        let outside = Directory::ensure_private(&temporary.path().join("outside")).unwrap();
        outside
            .create_new(OsStr::new("sentinel"))
            .unwrap()
            .write_all(b"outside bytes")
            .unwrap();
        symlink(outside.path(), root.path().join("link")).unwrap();

        let error = root.remove_tree().unwrap_err();
        assert_eq!(error.phase, PublicationPhase::Rejected);
        assert!(temporary.path().join("root/link").is_symlink());
        assert_eq!(
            std::fs::read(temporary.path().join("outside/sentinel")).unwrap(),
            b"outside bytes"
        );
    }

    #[test]
    fn checked_tree_removal_rejects_a_replaced_root_without_touching_it() {
        let temporary = tempfile::tempdir().unwrap();
        let root_path = temporary.path().join("root");
        let root = Directory::ensure_private(&root_path).unwrap();
        root.create_new(OsStr::new("original"))
            .unwrap()
            .write_all(b"original bytes")
            .unwrap();
        let parked = temporary.path().join("parked-original");
        std::fs::rename(&root_path, &parked).unwrap();
        let replacement = Directory::ensure_private(&root_path).unwrap();
        replacement
            .create_new(OsStr::new("replacement"))
            .unwrap()
            .write_all(b"replacement bytes")
            .unwrap();

        let error = root.remove_tree().unwrap_err();
        assert_eq!(error.phase, PublicationPhase::Rejected);
        assert_eq!(
            std::fs::read(root_path.join("replacement")).unwrap(),
            b"replacement bytes"
        );
        assert_eq!(
            std::fs::read(parked.join("original")).unwrap(),
            b"original bytes"
        );
    }

    #[test]
    fn checked_tree_removal_rejects_a_pinned_root_before_mutation() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("root");
        let root = Directory::ensure_private(&path).unwrap();
        root.create_new(OsStr::new("record"))
            .unwrap()
            .write_all(b"retained")
            .unwrap();
        let pinned = Directory::open(&path, Privacy::OwnerOnly, NameRetention::Pinned).unwrap();

        let error = pinned.remove_tree().unwrap_err();
        assert_eq!(error.phase, PublicationPhase::Rejected);
        assert_eq!(std::fs::read(path.join("record")).unwrap(), b"retained");
    }

    #[test]
    fn impossible_directory_move_preserves_native_error_and_source_identity() {
        let temporary = tempfile::tempdir().unwrap();
        let source = Directory::ensure_private(&temporary.path().join("source")).unwrap();
        source
            .create_new(OsStr::new("record"))
            .unwrap()
            .write_all(b"retained")
            .unwrap();
        let child = Directory::ensure_private(&source.path().join("child")).unwrap();
        // The preflight is valid, but the real OS cannot move an ancestor into
        // its own descendant. No injected OS error replaces this operation.
        let error = child
            .move_new_directory(&source, OsStr::new("impossible"))
            .unwrap_err();
        assert_eq!(error.operation, "native-move");
        assert_eq!(error.source_identity, Some(source.identity()));
        assert!(error.error().raw_os_error().is_some());
        assert_eq!(
            std::error::Error::source(&error)
                .unwrap()
                .downcast_ref::<io::Error>()
                .unwrap()
                .raw_os_error(),
            error.error().raw_os_error()
        );
        #[cfg(unix)]
        assert_eq!(error.phase, PublicationPhase::Rejected);
        #[cfg(windows)]
        assert_eq!(error.phase, PublicationPhase::Uncertain);
        assert!(error.to_string().contains("during native-move"));
        assert_eq!(
            Directory::open(source.path(), Privacy::OwnerOnly, NameRetention::Movable)
                .unwrap()
                .identity(),
            source.identity()
        );
        assert_eq!(
            std::fs::read(source.path().join("record")).unwrap(),
            b"retained"
        );
        assert!(!child.path().join("impossible").exists());
    }

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
                None,
                || {
                    Err(io::Error::other(
                        "controlled completion failure after actual native move",
                    ))
                },
            )
            .unwrap_err();
        assert_eq!(error.phase, PublicationPhase::Uncertain);
        assert_eq!(error.operation, "postmove-completion");
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
