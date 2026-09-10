//! Memory-owned composition of the checked platform filesystem primitives.
use anyhow::{Context, Result, ensure};
use kuru_platform::fs::{Directory, NameRetention, Privacy, Publication, PublicationPhase};
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
    let parent = parent(destination, Privacy::OwnerOnly, NameRetention::Movable)?;
    match parent.move_new_directory(source, name(destination)?) {
        Ok(moved) => Ok(moved),
        Err(error) if error.phase == PublicationPhase::Uncertain => {
            let moved = directory(destination)
                .with_context(|| format!("uncertain memory directory publication: {error}"))?;
            ensure!(
                moved.identity() == source.identity(),
                "uncertain memory publication has an unrelated destination; preserve both paths"
            );
            ensure!(
                matches!(fs::symlink_metadata(source.path()), Err(error) if error.kind() == std::io::ErrorKind::NotFound),
                "uncertain memory publication still has a source name; preserve both paths"
            );
            Ok(moved)
        }
        Err(error) => Err(error.into()),
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
}
