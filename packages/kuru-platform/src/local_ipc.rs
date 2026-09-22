//! Owner-private Unix sockets for local service connections.
//!
//! The directory is checked and retained; socket names are unique per service
//! generation. The filesystem permission boundary is other users, not other
//! processes running under the same account.

use crate::fs::{Directory, NameRetention, Privacy, validate_component};
use std::{
    ffi::{OsStr, OsString},
    fs, io,
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    path::PathBuf,
};
use tokio::net::{UnixListener, UnixStream};

/// A checked, short owner-private socket directory. Unix-domain socket names
/// have a fixed native path limit, so a configured data directory cannot be
/// used as the socket parent. `locator` is only a short routing hint; callers
/// must authenticate the complete project and live service generation.
pub fn prepare_short_directory(locator: &str) -> io::Result<Directory> {
    Directory::ensure_private(&short_directory_path(locator)?)
}

/// Open an existing short socket directory without creating it on a cold
/// client probe. Existing symlinks and nonprivate names are rejected.
pub fn open_short_directory(locator: &str) -> io::Result<Directory> {
    Directory::open(
        &short_directory_path(locator)?,
        Privacy::OwnerOnly,
        NameRetention::Pinned,
    )
}

fn short_directory_path(locator: &str) -> io::Result<PathBuf> {
    if locator.len() != 24
        || !locator
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid private service socket locator",
        ));
    }
    Ok(PathBuf::from("/tmp")
        .join(format!(
            "kuru-service-{}",
            rustix::process::geteuid().as_raw()
        ))
        .join(locator))
}

pub struct PrivateServiceListener {
    directory: Directory,
    name: OsString,
    listener: UnixListener,
    device: u64,
    inode: u64,
}

impl PrivateServiceListener {
    /// Bind a new generation name. An occupied name is never adopted or
    /// unlinked; the service owner chooses a fresh name after crash recovery.
    pub fn bind_at(directory: Directory, name: &OsStr) -> io::Result<Self> {
        validate_component(name)?;
        directory.revalidate()?;
        let private = Directory::open(directory.path(), Privacy::OwnerOnly, NameRetention::Pinned)?;
        if private.identity() != directory.identity() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "private service directory changed during bind",
            ));
        }
        let directory = private;
        let path = directory.path().join(name);
        let listener = UnixListener::bind(&path)?;
        let result = (|| {
            directory.revalidate()?;
            let metadata = fs::symlink_metadata(&path)?;
            if !metadata.file_type().is_socket() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "private service endpoint is not a socket",
                ));
            }
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
            directory.revalidate()?;
            let after = fs::symlink_metadata(&path)?;
            if !after.file_type().is_socket()
                || metadata.dev() != after.dev()
                || metadata.ino() != after.ino()
                || after.uid() != rustix::process::geteuid().as_raw()
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "private service endpoint changed during bind",
                ));
            }
            Ok((metadata.dev(), metadata.ino()))
        })();
        let (device, inode) = match result {
            Ok(identity) => identity,
            Err(error) => {
                drop(listener);
                return Err(error);
            }
        };
        Ok(Self {
            directory,
            name: name.to_owned(),
            listener,
            device,
            inode,
        })
    }

    pub fn path(&self) -> PathBuf {
        self.directory.path().join(&self.name)
    }

    pub async fn accept(&self) -> io::Result<UnixStream> {
        self.directory.revalidate()?;
        let (stream, _) = self.listener.accept().await?;
        self.directory.revalidate()?;
        Ok(stream)
    }
}

impl Drop for PrivateServiceListener {
    fn drop(&mut self) {
        if self.directory.revalidate().is_err() {
            return;
        }
        let path = self.path();
        if fs::symlink_metadata(&path).is_ok_and(|metadata| {
            metadata.file_type().is_socket()
                && metadata.dev() == self.device
                && metadata.ino() == self.inode
        }) {
            let _ = fs::remove_file(path);
        }
    }
}

/// Connect only through a currently checked private parent. Protocol
/// authentication remains the caller's responsibility.
pub async fn connect(directory: &Directory, name: &OsStr) -> io::Result<UnixStream> {
    validate_component(name)?;
    directory.revalidate()?;
    let private = Directory::open(directory.path(), Privacy::OwnerOnly, NameRetention::Pinned)?;
    if private.identity() != directory.identity() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "private service directory changed before connect",
        ));
    }
    let path = private.path().join(name);
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.file_type().is_socket()
        || metadata.mode() & 0o077 != 0
        || metadata.uid() != rustix::process::geteuid().as_raw()
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "private service endpoint is not an owner-private socket",
        ));
    }
    let stream = UnixStream::connect(&path).await?;
    private.revalidate()?;
    Ok(stream)
}
