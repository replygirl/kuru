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

pub(crate) fn directory(path: &Path) -> Result<Directory> {
    Ok(Directory::open(
        path,
        Privacy::OwnerOnly,
        NameRetention::Movable,
    )?)
}

pub(crate) fn private_dir(path: &Path) -> Result<()> {
    Directory::ensure_private(path)?;
    Ok(())
}

pub(crate) fn parent(path: &Path, privacy: Privacy, retention: NameRetention) -> Result<Directory> {
    Ok(Directory::open(
        path.parent().context("memory file needs a parent")?,
        privacy,
        retention,
    )?)
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
    let stage = Directory::ensure_private(&parent.path().join("staging"))?;
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

/// Preserve a stopped directory's identity through an uncertain native move.
pub(crate) fn move_directory(source: &Directory, destination: &Path) -> Result<Directory> {
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
) -> Result<Directory> {
    let parent = parent(destination, Privacy::OwnerOnly, NameRetention::Movable)?;
    match publish(&parent, source, name(destination)?) {
        Ok(moved) => Ok(moved),
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
            reconcile().map_err(|secondary| {
                // Preserve the original typed publication/OS error in the cause
                // chain. These bounded fresh observations never authorize a retry.
                error.context(format!(
                    "memory directory reconciliation failed: {secondary:#}; held_source={:?}; source={}; destination={}; preserve both paths",
                    source.identity(),
                    observe_directory(source.path(), source.identity()),
                    observe_directory(destination, source.identity()),
                ))
            })
        }
        Err((_, error)) => Err(error),
    }
}

fn observe_directory(path: &Path, expected: FileIdentity) -> String {
    match Directory::open(path, Privacy::OwnerOnly, NameRetention::Movable) {
        Ok(directory) if directory.identity() == expected => "same-identity".into(),
        Ok(directory) => format!("different-identity({:?})", directory.identity()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => "absent".into(),
        Err(error) => format!(
            "query-error(kind={:?}, os={:?})",
            error.kind(),
            error.raw_os_error()
        ),
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
    move_directory_with(source, destination, |parent, source, name| {
        let moved = parent
            .move_new_directory(source, name)
            .map_err(|error| (error.phase, anyhow::Error::from(error)))?;
        observer(&moved).map_err(|error| (PublicationPhase::Uncertain, error))?;
        Ok(moved)
    })
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
