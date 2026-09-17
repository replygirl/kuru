//! Memory-owned composition of the checked platform filesystem primitives.
use anyhow::{Context, Result, ensure};
use kuru_platform::fs::{
    Directory, FileIdentity, NameRetention, Privacy, Publication, PublicationPhase,
};
use std::{
    ffi::OsStr,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[cfg(windows)]
use kuru_platform::fs::RemovalError;
#[cfg(windows)]
use std::{
    io, thread,
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

pub(crate) fn directory(path: &Path) -> Result<Directory> {
    open_directory(path, Privacy::OwnerOnly, NameRetention::Movable)
}

/// Open a checked directory while preserving the caller's privacy and name
/// retention contract. Memory owns the actionable Unix privacy diagnostic;
/// the platform continues to provide the checked filesystem fact.
pub(crate) fn open_directory(
    path: &Path,
    privacy: Privacy,
    retention: NameRetention,
) -> Result<Directory> {
    Directory::open(path, privacy, retention)
        .map_err(anyhow::Error::from)
        .map_err(|error| private_directory_error(path, privacy, error))
}

pub(crate) fn ensure_private_directory(path: &Path) -> Result<Directory> {
    Directory::ensure_private(path)
        .map_err(anyhow::Error::from)
        .map_err(|error| private_directory_error(path, Privacy::OwnerOnly, error))
}

pub(crate) fn private_dir(path: &Path) -> Result<()> {
    ensure_private_directory(path)?;
    Ok(())
}

pub(crate) fn parent(path: &Path, privacy: Privacy, retention: NameRetention) -> Result<Directory> {
    open_directory(
        path.parent().context("memory file needs a parent")?,
        privacy,
        retention,
    )
}

fn private_directory_error(path: &Path, privacy: Privacy, error: anyhow::Error) -> anyhow::Error {
    #[cfg(not(unix))]
    let _ = (path, privacy);
    #[cfg(unix)]
    if privacy == Privacy::OwnerOnly
        && let Ok(directory) = fs::symlink_metadata(path)
        && error
            .downcast_ref::<std::io::Error>()
            .is_some_and(|error| error.kind() == std::io::ErrorKind::PermissionDenied)
        && directory.is_dir()
        && directory.uid() == nix::unistd::geteuid().as_raw()
        && directory.permissions().mode() & 0o077 != 0
    {
        return error.context(format!(
            "memory data directory {path:?} is not owner-private; restrict this exact directory to mode 0700 (for example with chmod, using shell quoting) and retry"
        ));
    }
    error
}

pub(crate) fn name(path: &Path) -> Result<&OsStr> {
    path.file_name().context("memory file needs a literal name")
}

pub(crate) fn read(path: &Path, privacy: Privacy) -> Result<(Directory, File)> {
    let parent = parent(path, privacy, NameRetention::Movable)?;
    let file = parent.read(name(path)?)?;
    Ok((parent, file))
}

pub(crate) fn read_bytes(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let (parent, mut file) = read(path, Privacy::OwnerOnly)?;
    let mut bytes = Vec::new();
    (&mut file).take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "private memory file exceeds size limit"
    );
    parent.verify(name(path)?, &file)?;
    Ok(bytes)
}

pub(crate) fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = parent(path, Privacy::OwnerOnly, NameRetention::Movable)?;
    write_in(&parent, name(path)?, bytes)
}

/// Dolt rewrites this config with ordinary Unix file permissions. Its parent
/// remains private; replace the checked ordinary file with a freshly private
/// candidate instead of silently changing an existing object's access policy.
pub(crate) fn write_dolt_config(path: &Path, bytes: &[u8]) -> Result<()> {
    let private = parent(path, Privacy::OwnerOnly, NameRetention::Movable)?;
    #[cfg(unix)]
    {
        let destination =
            Directory::open(private.path(), Privacy::Inherited, NameRetention::Movable)?;
        write_in(&destination, name(path)?, bytes)?;
    }
    #[cfg(windows)]
    write_in(&private, name(path)?, bytes)?;
    let file = private.read(name(path)?)?;
    private.verify(name(path)?, &file)?;
    Ok(())
}

fn write_in(parent: &Directory, name: &OsStr, bytes: &[u8]) -> Result<()> {
    let stage = ensure_private_directory(&parent.path().join("staging"))?;
    let temporary = format!("record-{}.tmp", uuid::Uuid::new_v4());
    let mut file = stage.create_new(OsStr::new(&temporary))?;
    file.write_all(bytes)?;
    parent
        .publish_file(
            &stage,
            OsStr::new(&temporary),
            &file,
            name,
            Publication::ReplaceRegular,
        )
        .with_context(|| {
            format!(
                "publish private memory record {}; preserve staging on uncertainty",
                parent.path().join(name).display()
            )
        })?;
    Ok(())
}

/// The checked outcome of a stopped directory move.
///
/// The no-move variant is intentionally private to memory provisioning. Other
/// callers retain the established erased `Result<Directory>` contract.
pub(crate) enum DirectoryMove {
    Moved(Directory),
    ProvenNoMove(anyhow::Error),
}

/// Preserve a stopped directory's identity through an uncertain native move.
pub(crate) fn move_directory(source: &Directory, destination: &Path) -> Result<Directory> {
    match move_directory_checked(source, destination)? {
        DirectoryMove::Moved(directory) => Ok(directory),
        DirectoryMove::ProvenNoMove(error) => Err(error),
    }
}

/// Preserve a stopped directory's identity while exposing the one checked
/// no-move outcome that runtime activation may recover on Windows.
pub(crate) fn move_directory_checked(
    source: &Directory,
    destination: &Path,
) -> Result<DirectoryMove> {
    move_directory_with(source, destination, |parent, source, name| {
        parent
            .move_new_directory(source, name)
            .map_err(|error| (error.phase, anyhow::Error::from(error)))
    })
}

fn move_directory_with(
    source: &Directory,
    destination: &Path,
    publish: impl FnOnce(
        &Directory,
        &Directory,
        &OsStr,
    ) -> std::result::Result<Directory, (PublicationPhase, anyhow::Error)>,
) -> Result<DirectoryMove> {
    let parent = parent(destination, Privacy::OwnerOnly, NameRetention::Movable)?;
    let destination_name = name(destination)?;
    match publish(&parent, source, destination_name) {
        Ok(moved) => Ok(DirectoryMove::Moved(moved)),
        Err((PublicationPhase::Uncertain, error)) => {
            let reconcile = || -> Result<Directory> {
                let moved = directory(destination).context("open publication destination")?;
                ensure!(
                    moved.identity() == source.identity(),
                    "publication destination has an unrelated identity"
                );
                ensure!(
                    matches!(fs::symlink_metadata(source.path()), Err(error) if error.kind() == std::io::ErrorKind::NotFound),
                    "publication source name is not absent"
                );
                Ok(moved)
            };
            match reconcile() {
                Ok(moved) => Ok(DirectoryMove::Moved(moved)),
                Err(secondary) => {
                    // Only an unchanged held source and an absent destination
                    // prove that this particular move did not occur. Every
                    // other observation preserves the original uncertain error.
                    let no_move = source.revalidate().is_ok()
                        && parent.revalidate().is_ok()
                        && matches!(parent.read(destination_name), Err(error) if error.kind() == std::io::ErrorKind::NotFound);
                    if no_move {
                        let mut error = error;
                        let Some(publication) =
                            error.downcast_mut::<kuru_platform::fs::PublicationError>()
                        else {
                            return Err(error.context(
                                "uncertain directory move omitted its typed publication error",
                            ));
                        };
                        publication.phase = PublicationPhase::Rejected;
                        return Ok(DirectoryMove::ProvenNoMove(error.context(
                            "memory directory reconciliation proved the held source remains and the destination is absent",
                        )));
                    }
                    // Preserve the original typed publication/OS error in the
                    // cause chain. These bounded fresh observations never
                    // authorize a retry.
                    Err(error.context(format!(
                        "memory directory reconciliation failed: {secondary:#}; held_source={:?}; source={}; destination={}; preserve both paths",
                        source.identity(),
                        observe_directory(source.path(), source.identity()),
                        observe_directory(destination, source.identity()),
                    )))
                }
            }
        }
        Err((_, error)) => Err(error),
    }
}

fn observe_directory(path: &Path, expected: FileIdentity) -> String {
    match open_directory(path, Privacy::OwnerOnly, NameRetention::Movable) {
        Ok(directory) if directory.identity() == expected => "same-identity".into(),
        Ok(directory) => format!("different-identity({:?})", directory.identity()),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            "absent".into()
        }
        Err(error) => {
            let error = error.downcast_ref::<std::io::Error>();
            format!(
                "query-error(kind={:?}, os={:?})",
                error
                    .map(std::io::Error::kind)
                    .unwrap_or(std::io::ErrorKind::Other),
                error.and_then(std::io::Error::raw_os_error)
            )
        }
    }
}

/// Test-only completion observer after the actual checked native move. A
/// controlled completion error enters the ordinary identity reconciliation;
/// the observer never replaces the filesystem operation or runs in production.
#[cfg(test)]
pub(crate) fn move_directory_observed(
    source: &Directory,
    destination: &Path,
    observer: impl FnOnce(&Directory) -> Result<()>,
) -> Result<Directory> {
    match move_directory_with(source, destination, |parent, source, name| {
        let moved = parent
            .move_new_directory(source, name)
            .map_err(|error| (error.phase, anyhow::Error::from(error)))?;
        observer(&moved).map_err(|error| (PublicationPhase::Uncertain, error))?;
        Ok(moved)
    })? {
        DirectoryMove::Moved(directory) => Ok(directory),
        DirectoryMove::ProvenNoMove(error) => Err(error),
    }
}

/// TempDir owns only the disposable outer container. Private data is created
/// under a protected child, including on Windows where tempfile inherits ACLs.
pub struct PrivateTemp {
    _container: tempfile::TempDir,
    path: PathBuf,
}

impl PrivateTemp {
    pub(crate) fn new(prefix: &str, parent: Option<&Path>) -> Result<Self> {
        let mut builder = tempfile::Builder::new();
        builder.prefix(prefix);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(fs::Permissions::from_mode(0o700));
        }
        let container = match parent {
            Some(parent) => builder.tempdir_in(parent)?,
            None => builder.tempdir()?,
        };
        let path = container.path().join("private");
        private_dir(&path)?;
        Ok(Self {
            _container: container,
            path,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Preserve verified engine activation evidence; this does not create an
    /// automatic database recovery action or retain any running process.
    pub(crate) fn keep(self) -> PathBuf {
        let _container_path = self._container.keep();
        self.path
    }

    /// Report removal failures after a successfully published private stage.
    pub(crate) fn close(self) -> Result<()> {
        let Self { _container, path } = self;
        #[cfg(windows)]
        {
            let outer_path = _container.path().to_owned();
            let kept = _container.keep();
            ensure!(
                kept == outer_path,
                "temporary stage container path changed before cleanup"
            );
            let outer = open_directory(&outer_path, Privacy::Inherited, NameRetention::Movable)?;
            let outer_identity = outer.identity();
            drop(outer);
            let child = open_directory(&path, Privacy::OwnerOnly, NameRetention::Movable)?;
            let child_identity = child.identity();
            return close_windows_private_stage(
                &outer_path,
                outer_identity,
                &path,
                child_identity,
                child,
            )
            .with_context(|| format!("remove private temporary stage at {}", path.display()));
        }
        #[cfg(not(windows))]
        _container
            .close()
            .with_context(|| format!("remove private temporary stage at {}", path.display()))
    }
}

#[cfg(windows)]
const CLEANUP_RETRY_LIMIT: Duration = Duration::from_secs(2);
#[cfg(windows)]
const CLEANUP_RETRY_SPACING: Duration = Duration::from_millis(20);

#[cfg(windows)]
fn close_windows_private_stage(
    outer_path: &Path,
    outer_identity: FileIdentity,
    child_path: &Path,
    child_identity: FileIdentity,
    initial_child: Directory,
) -> Result<()> {
    close_windows_private_stage_with(
        outer_path,
        outer_identity,
        child_path,
        child_identity,
        initial_child,
        |_| {},
    )
}

#[cfg(windows)]
fn close_windows_private_stage_with(
    outer_path: &Path,
    outer_identity: FileIdentity,
    child_path: &Path,
    child_identity: FileIdentity,
    initial_child: Directory,
    after_first_recoverable_error: impl FnOnce(Option<PublicationPhase>),
) -> Result<()> {
    let mut retry_deadline = None;
    let mut first_error = None;
    let mut after_first_recoverable_error = Some(after_first_recoverable_error);
    let mut child = Some(initial_child);
    loop {
        exact_outer(outer_path, outer_identity)?;
        let stage = match child.take() {
            Some(stage) => stage,
            None => match child_state(child_path, child_identity)? {
                ChildState::Same(stage) => stage,
                ChildState::Absent => break,
                ChildState::Pending => {
                    if first_error.is_none() {
                        first_error = Some(anyhow::anyhow!(
                            "private child became pending after checked removal"
                        ));
                    }
                    wait_for_cleanup_retry(
                        cleanup_deadline(&mut retry_deadline),
                        &mut first_error,
                    )?;
                    continue;
                }
            },
        };
        let state = match stage.remove_tree() {
            Ok(()) => child_state(child_path, child_identity)?,
            Err(error) if recoverable_child_removal(&error) => {
                cleanup_deadline(&mut retry_deadline);
                let phase = error.phase;
                if first_error.is_none() {
                    first_error = Some(anyhow::Error::new(error));
                }
                if let Some(observer) = after_first_recoverable_error.take() {
                    observer(Some(phase));
                }
                child_state(child_path, child_identity)?
            }
            Err(error) => return Err(anyhow::Error::new(error)),
        };
        match state {
            ChildState::Absent => break,
            ChildState::Same(stage) => child = Some(stage),
            ChildState::Pending => {}
        }
        if first_error.is_none() {
            first_error = Some(anyhow::anyhow!(
                "private temporary stage remained after checked removal"
            ));
        }
        wait_for_cleanup_retry(cleanup_deadline(&mut retry_deadline), &mut first_error)?;
    }
    loop {
        match child_state(child_path, child_identity)? {
            ChildState::Absent => {}
            ChildState::Pending => {
                if first_error.is_none() {
                    first_error = Some(anyhow::anyhow!(
                        "private child became pending after checked removal"
                    ));
                }
                wait_for_cleanup_retry(cleanup_deadline(&mut retry_deadline), &mut first_error)?;
                continue;
            }
            ChildState::Same(_) => {
                return Err(anyhow::anyhow!(
                    "private temporary stage remained after checked removal"
                ));
            }
        }
        match outer_state(outer_path, outer_identity)? {
            OuterState::Absent => return Ok(()),
            OuterState::Pending(error) => {
                if first_error.is_none() {
                    first_error = Some(error);
                }
                wait_for_cleanup_retry(cleanup_deadline(&mut retry_deadline), &mut first_error)?;
                continue;
            }
            OuterState::Same => {}
        }
        match fs::remove_dir(outer_path) {
            Ok(()) => match outer_state(outer_path, outer_identity)? {
                OuterState::Absent => return Ok(()),
                OuterState::Same => {
                    if first_error.is_none() {
                        first_error = Some(anyhow::anyhow!(
                            "temporary stage container remained after removal"
                        ));
                    }
                    wait_for_cleanup_retry(
                        cleanup_deadline(&mut retry_deadline),
                        &mut first_error,
                    )?;
                }
                OuterState::Pending(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                    wait_for_cleanup_retry(
                        cleanup_deadline(&mut retry_deadline),
                        &mut first_error,
                    )?;
                }
            },
            Err(error) if matches!(error.raw_os_error(), Some(32 | 145)) => {
                if first_error.is_none() {
                    first_error = Some(anyhow::Error::from(error));
                }
                if let Some(observer) = after_first_recoverable_error.take() {
                    observer(None);
                }
                wait_for_cleanup_retry(cleanup_deadline(&mut retry_deadline), &mut first_error)?;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

#[cfg(windows)]
enum ChildState {
    Same(Directory),
    Absent,
    Pending,
}

#[cfg(windows)]
enum OuterState {
    Same,
    Absent,
    Pending(anyhow::Error),
}

#[cfg(windows)]
fn exact_outer(path: &Path, expected: FileIdentity) -> Result<()> {
    let outer = open_directory(path, Privacy::Inherited, NameRetention::Movable)?;
    ensure!(
        outer.identity() == expected,
        "temporary stage container identity changed during cleanup"
    );
    Ok(())
}

#[cfg(windows)]
fn child_state(path: &Path, expected: FileIdentity) -> Result<ChildState> {
    match open_directory(path, Privacy::OwnerOnly, NameRetention::Movable) {
        Ok(child) => {
            ensure!(
                child.identity() == expected,
                "private temporary stage identity changed during cleanup"
            );
            Ok(ChildState::Same(child))
        }
        Err(error)
            if error
                .downcast_ref::<io::Error>()
                .is_some_and(|error| error.kind() == io::ErrorKind::NotFound) =>
        {
            Ok(ChildState::Absent)
        }
        Err(error) if pending_open_error(&error) => Ok(ChildState::Pending),
        Err(error) => Err(error),
    }
}

#[cfg(windows)]
fn outer_state(path: &Path, expected: FileIdentity) -> Result<OuterState> {
    match open_directory(path, Privacy::Inherited, NameRetention::Movable) {
        Ok(outer) => {
            ensure!(
                outer.identity() == expected,
                "temporary stage container identity changed during cleanup"
            );
            Ok(OuterState::Same)
        }
        Err(error)
            if error
                .downcast_ref::<io::Error>()
                .is_some_and(|error| error.kind() == io::ErrorKind::NotFound) =>
        {
            Ok(OuterState::Absent)
        }
        Err(error) if pending_open_error(&error) => Ok(OuterState::Pending(error)),
        Err(error) => Err(error),
    }
}

/// A rejected removal changed nothing, so a sharing violation or a denied
/// delete of a name Windows still holds - a delete-pending entry, or the image
/// of the just-executed private probe copy - may be reconciled against the same
/// retained identity. This matches `pending_open_error` and bounded activation
/// recovery, which already treat native error 5 as a transient native holder.
#[cfg(windows)]
fn recoverable_child_removal(error: &RemovalError) -> bool {
    error.phase == PublicationPhase::Uncertain
        || (error.phase == PublicationPhase::Rejected
            && matches!(removal_error_code(error), Some(5 | 32)))
}

#[cfg(windows)]
fn removal_error_code(error: &RemovalError) -> Option<i32> {
    std::error::Error::source(error)
        .and_then(|source| source.downcast_ref::<io::Error>())
        .and_then(io::Error::raw_os_error)
}

#[cfg(windows)]
fn pending_open_error(error: &anyhow::Error) -> bool {
    error.downcast_ref::<io::Error>().is_some_and(|error| {
        matches!(error.raw_os_error(), Some(5 | 32))
            || error.kind() == io::ErrorKind::PermissionDenied
    })
}

#[cfg(windows)]
fn cleanup_deadline(deadline: &mut Option<Instant>) -> Instant {
    *deadline.get_or_insert_with(|| Instant::now() + CLEANUP_RETRY_LIMIT)
}

#[cfg(windows)]
fn wait_for_cleanup_retry(
    deadline: Instant,
    first_error: &mut Option<anyhow::Error>,
) -> Result<()> {
    if Instant::now() >= deadline {
        return Err(first_error
            .take()
            .expect("a bounded cleanup retry retains its first cause")
            .context("private temporary stage cleanup exhausted its bounded recovery; preserve the published stage for inspection"));
    }
    thread::sleep(
        deadline
            .saturating_duration_since(Instant::now())
            .min(CLEANUP_RETRY_SPACING),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    fn close_private_temp_observed(
        stage: PrivateTemp,
        after_first_recoverable_error: impl FnOnce(Option<PublicationPhase>),
    ) -> Result<()> {
        let PrivateTemp { _container, path } = stage;
        let outer_path = _container.path().to_owned();
        let kept = _container.keep();
        ensure!(
            kept == outer_path,
            "temporary stage container path changed before cleanup"
        );
        let outer = open_directory(&outer_path, Privacy::Inherited, NameRetention::Movable)?;
        let outer_identity = outer.identity();
        drop(outer);
        let child = open_directory(&path, Privacy::OwnerOnly, NameRetention::Movable)?;
        let child_identity = child.identity();
        close_windows_private_stage_with(
            &outer_path,
            outer_identity,
            &path,
            child_identity,
            child,
            after_first_recoverable_error,
        )
    }

    #[cfg(windows)]
    fn held_private_file(path: &Path) -> File {
        use std::os::windows::fs::OpenOptionsExt;

        fs::write(path, b"fixture cleanup blocker").unwrap();
        fs::OpenOptions::new()
            .read(true)
            .share_mode(0x3)
            .open(path)
            .unwrap()
    }

    /// Hold a delete-on-close handle so the name stays present but
    /// delete-pending. Every later checked open then reports native error 5,
    /// the same result Windows gives for the live image of a just-executed
    /// private probe copy.
    #[cfg(windows)]
    fn delete_pending_private_file(path: &Path) -> File {
        use std::os::windows::fs::OpenOptionsExt;

        const DELETE: u32 = 0x0001_0000;
        const GENERIC_READ: u32 = 0x8000_0000;
        const SHARE_READ_WRITE_DELETE: u32 = 0x7;
        const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;

        fs::write(path, b"fixture delete-pending blocker").unwrap();
        fs::OpenOptions::new()
            .access_mode(GENERIC_READ | DELETE)
            .share_mode(SHARE_READ_WRITE_DELETE)
            .custom_flags(FILE_FLAG_DELETE_ON_CLOSE)
            .open(path)
            .unwrap()
    }

    #[cfg(windows)]
    #[test]
    fn private_temp_retries_a_denied_child_delete_after_the_holder_releases() {
        let root = tempfile::tempdir().unwrap();
        let stage = PrivateTemp::new("memory-cleanup-denied-", Some(root.path())).unwrap();
        let outer = stage.path().parent().unwrap().to_owned();
        let stage_path = stage.path().to_owned();
        let stage_identity = directory(&stage_path).unwrap().identity();
        let blocker = delete_pending_private_file(&stage_path.join("pending"));
        let mut blocker = Some(blocker);
        let mut observed = None;

        close_private_temp_observed(stage, |phase| {
            observed = phase;
            assert_eq!(directory(&stage_path).unwrap().identity(), stage_identity);
            drop(blocker.take());
        })
        .unwrap();

        assert_eq!(observed, Some(PublicationPhase::Rejected));
        assert!(
            !outer.exists(),
            "the exact disposable outer stage is removed after a denied child delete is reconciled"
        );
    }

    #[cfg(windows)]
    #[test]
    fn private_temp_retries_an_observed_os32_after_the_holder_releases() {
        let root = tempfile::tempdir().unwrap();
        let stage = PrivateTemp::new("memory-cleanup-recover-", Some(root.path())).unwrap();
        let outer = stage.path().parent().unwrap().to_owned();
        let stage_path = stage.path().to_owned();
        let stage_identity = directory(&stage_path).unwrap().identity();
        let blocker = held_private_file(&stage.path().join("held"));
        let mut blocker = Some(blocker);
        let mut observed = None;

        close_private_temp_observed(stage, |phase| {
            observed = phase;
            assert_eq!(directory(&stage_path).unwrap().identity(), stage_identity);
            drop(blocker.take());
        })
        .unwrap();

        assert_eq!(observed, Some(PublicationPhase::Rejected));
        assert!(
            !outer.exists(),
            "the exact disposable outer stage is removed after checked recovery"
        );
    }

    #[cfg(windows)]
    #[test]
    fn private_temp_reconciles_pending_nested_delete_after_the_holder_releases() {
        let root = tempfile::tempdir().unwrap();
        let stage = PrivateTemp::new("memory-cleanup-pending-", Some(root.path())).unwrap();
        let outer = stage.path().parent().unwrap().to_owned();
        let probe = stage.path().join("probe");
        private_dir(&probe).unwrap();
        fs::write(probe.join("record"), b"fixture-only pending descendant").unwrap();
        let holder = Directory::open(&probe, Privacy::OwnerOnly, NameRetention::Movable).unwrap();
        let mut holder = Some(holder);
        let mut observed = None;

        close_private_temp_observed(stage, |phase| {
            observed = phase;
            drop(holder.take());
        })
        .unwrap();

        assert_eq!(observed, Some(PublicationPhase::Uncertain));
        assert!(
            !outer.exists(),
            "the exact outer stage is removed after pending-name reconciliation"
        );
    }

    #[cfg(windows)]
    #[test]
    fn private_temp_refuses_a_replacement_after_an_observed_os32() {
        let root = tempfile::tempdir().unwrap();
        let stage = PrivateTemp::new("memory-cleanup-replacement-", Some(root.path())).unwrap();
        let stage_path = stage.path().to_owned();
        let outer = stage_path.parent().unwrap().to_owned();
        let original = directory(&stage_path).unwrap().identity();
        let blocker = held_private_file(&stage_path.join("held"));
        let mut blocker = Some(blocker);

        let error = close_private_temp_observed(stage, |_| {
            drop(blocker.take());
            fs::remove_dir_all(&stage_path).unwrap();
            private_dir(&stage_path).unwrap();
        })
        .unwrap_err();

        assert!(
            format!("{error:#}")
                .contains("private temporary stage identity changed during cleanup"),
            "replacement must be refused rather than removed: {error:#}"
        );
        assert!(stage_path.is_dir());
        assert_ne!(directory(&stage_path).unwrap().identity(), original);
        fs::remove_dir_all(&outer).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn non_store_private_directory_failures_keep_native_diagnostics() {
        let error = private_directory_error(
            Path::new(r"C:\\kuru-memory-non-store"),
            Privacy::OwnerOnly,
            std::io::Error::from(std::io::ErrorKind::PermissionDenied).into(),
        );
        let text = format!("{error:#}");
        assert!(!text.contains("mode 0700"), "{text}");
        assert!(!text.contains("chmod"), "{text}");
        assert!(!text.contains("Windows file security"), "{text}");
        assert_eq!(
            error
                .downcast_ref::<std::io::Error>()
                .map(std::io::Error::kind),
            Some(std::io::ErrorKind::PermissionDenied),
            "{text}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn owner_private_boundaries_offer_a_safe_remedy_without_mutation() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

        let root = PrivateTemp::new("memory-private-boundary-", None).unwrap();
        let public = root.path().join("public");
        fs::create_dir(&public).unwrap();
        let sentinel = public.join("keep");
        fs::write(&sentinel, b"leave this directory unchanged").unwrap();
        fs::set_permissions(&public, fs::Permissions::from_mode(0o755)).unwrap();

        for error in [
            directory(&public).unwrap_err(),
            open_directory(&public, Privacy::OwnerOnly, NameRetention::Pinned).unwrap_err(),
            ensure_private_directory(&public).unwrap_err(),
            parent(
                &public.join("record"),
                Privacy::OwnerOnly,
                NameRetention::Movable,
            )
            .unwrap_err(),
        ] {
            let text = format!("{error:#}");
            assert!(text.contains("memory data directory"), "{text}");
            assert!(text.contains("mode 0700"), "{text}");
            assert!(text.contains(&public.display().to_string()), "{text}");
            assert_eq!(
                error
                    .downcast_ref::<std::io::Error>()
                    .map(std::io::Error::kind),
                Some(std::io::ErrorKind::PermissionDenied),
                "{text}"
            );
        }
        assert_eq!(
            fs::metadata(&public).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert_eq!(
            fs::read(&sentinel).unwrap(),
            b"leave this directory unchanged"
        );

        let link = root.path().join("linked");
        symlink(root.path(), &link).unwrap();
        let text = format!("{:#}", directory(&link).unwrap_err());
        assert!(!text.contains("mode 0700"), "{text}");

        let foreign = Path::new("/");
        if fs::symlink_metadata(foreign).unwrap().uid() != nix::unistd::geteuid().as_raw() {
            let text = format!("{:#}", directory(foreign).unwrap_err());
            assert!(!text.contains("mode 0700"), "{text}");
        }
    }

    #[test]
    fn untyped_uncertain_no_move_preserves_its_original_error() {
        let root = PrivateTemp::new("memory-untyped-move-", None).unwrap();
        let source_path = root.path().join("source");
        let source = Directory::ensure_private(&source_path).unwrap();
        let destination = root.path().join("active");

        let error = match move_directory_with(&source, &destination, |_, _, _| {
            Err((
                PublicationPhase::Uncertain,
                std::io::Error::from_raw_os_error(5).into(),
            ))
        }) {
            Ok(_) => panic!("untyped uncertain errors must not expose a no-move marker"),
            Err(error) => error,
        };
        assert_eq!(
            error
                .downcast_ref::<std::io::Error>()
                .unwrap()
                .raw_os_error(),
            Some(5)
        );
        assert!(format!("{error:#}").contains("omitted its typed publication error"));
        assert_eq!(
            directory(&source_path).unwrap().identity(),
            source.identity()
        );
        assert!(!destination.exists());
    }

    #[test]
    fn unresolved_real_move_reports_both_names_and_preserves_original_error() {
        let root = PrivateTemp::new("memory-move-observation-", None).unwrap();
        let source_path = root.path().join("source");
        let source = Directory::ensure_private(&source_path).unwrap();
        write(&source_path.join("record"), b"original committed bytes").unwrap();
        let destination = root.path().join("active");
        let identity = source.identity();
        let error = move_directory_observed(&source, &destination, |moved| {
            assert_eq!(moved.identity(), identity);
            // Simulate a conflicting new occupant only after the actual native
            // move. Reconciliation must preserve both objects and refuse success.
            Directory::ensure_private(&source_path)?;
            write(&source_path.join("record"), b"unrelated new occupant")?;
            Err(std::io::Error::from_raw_os_error(5).into())
        })
        .unwrap_err();
        assert_eq!(
            error
                .downcast_ref::<std::io::Error>()
                .unwrap()
                .raw_os_error(),
            Some(5)
        );
        let diagnostic = format!("{error:#}");
        assert!(diagnostic.contains("publication source name is not absent"));
        assert!(diagnostic.contains("source=different-identity("));
        assert!(diagnostic.contains("destination=same-identity"));
        assert!(diagnostic.contains("os error 5"));
        assert_eq!(directory(&destination).unwrap().identity(), identity);
        assert_ne!(directory(&source_path).unwrap().identity(), identity);
        assert_eq!(
            fs::read(destination.join("record")).unwrap(),
            b"original committed bytes"
        );
        assert_eq!(
            fs::read(source_path.join("record")).unwrap(),
            b"unrelated new occupant"
        );
    }
}
