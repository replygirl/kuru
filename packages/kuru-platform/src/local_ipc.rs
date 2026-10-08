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

#[cfg(test)]
mod refusal_contracts {
    use super::*;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[test]
    fn short_directory_cold_probe_preserves_the_shared_prefix_and_only_owns_its_leaf() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            ^ (u128::from(std::process::id()) << 64);
        let locator = format!("{nonce:024x}");
        let path = short_directory_path(&locator).unwrap();
        assert!(!path.exists());
        let prefix = path.parent().unwrap();
        let prefix_existed = prefix.exists();
        assert_eq!(
            open_short_directory(&locator).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert!(!path.exists(), "a cold client must not create state");
        assert_eq!(prefix.exists(), prefix_existed);
        let retained_prefix = prefix_existed
            .then(|| Directory::open(prefix, Privacy::OwnerOnly, NameRetention::Pinned).unwrap());
        let prefix_mode =
            prefix_existed.then(|| fs::metadata(prefix).unwrap().permissions().mode());
        // Product preparation may create its checked private prefix when
        // absent. The fixture never chmods or removes that shared prefix.
        let prepared = prepare_short_directory(&locator).unwrap();
        let reopened = open_short_directory(&locator).unwrap();
        assert_eq!(prepared.identity(), reopened.identity());
        drop(reopened);
        prepared.remove_tree().unwrap();
        assert!(!path.exists());
        assert!(prefix.exists());
        if let Some(retained_prefix) = retained_prefix {
            retained_prefix.revalidate().unwrap();
            assert_eq!(
                fs::metadata(prefix).unwrap().permissions().mode(),
                prefix_mode.unwrap()
            );
        }
    }

    #[tokio::test]
    async fn invalid_endpoint_names_and_stale_sockets_never_adopt_or_remove_state() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = Directory::ensure_private(&temporary.path().join("private")).unwrap();
        let path = directory.path().to_path_buf();
        fs::write(path.join("adjacent"), b"unchanged").unwrap();
        fs::set_permissions(path.join("adjacent"), fs::Permissions::from_mode(0o600)).unwrap();
        for name in ["", "..", "nested/endpoint", "endpoint\0outside"] {
            assert_eq!(
                connect(&directory, OsStr::new(name))
                    .await
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidInput
            );
            assert_eq!(
                PrivateServiceListener::bind_at(
                    Directory::ensure_private(&path).unwrap(),
                    OsStr::new(name),
                )
                .err()
                .unwrap()
                .kind(),
                io::ErrorKind::InvalidInput
            );
        }
        assert_eq!(
            connect(&directory, OsStr::new("missing"))
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
        let endpoint = path.join("stale.sock");
        let native = UnixListener::bind(&endpoint).unwrap();
        fs::set_permissions(&endpoint, fs::Permissions::from_mode(0o600)).unwrap();
        drop(native);
        let error = tokio::time::timeout(
            Duration::from_secs(5),
            connect(&directory, OsStr::new("stale.sock")),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::ConnectionRefused);
        assert!(
            fs::symlink_metadata(&endpoint)
                .unwrap()
                .file_type()
                .is_socket()
        );
        assert!(
            PrivateServiceListener::bind_at(
                Directory::ensure_private(&path).unwrap(),
                OsStr::new("stale.sock"),
            )
            .is_err()
        );
        assert_eq!(fs::read(path.join("adjacent")).unwrap(), b"unchanged");
    }

    #[tokio::test]
    async fn listener_drop_preserves_another_native_socket_at_the_same_name() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = Directory::ensure_private(&temporary.path().join("private")).unwrap();
        let listener =
            PrivateServiceListener::bind_at(directory, OsStr::new("generation.sock")).unwrap();
        let original = listener.path();
        let moved = original.with_file_name("displaced.sock");
        fs::rename(&original, &moved).unwrap();
        let replacement = UnixListener::bind(&original).unwrap();
        let identity = fs::symlink_metadata(&original).unwrap().ino();
        drop(listener);
        assert_eq!(fs::symlink_metadata(&original).unwrap().ino(), identity);
        assert!(
            fs::symlink_metadata(&moved)
                .unwrap()
                .file_type()
                .is_socket()
        );
        drop(replacement);
    }
}
