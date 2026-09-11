//! Native fixture operations; no application policy fallback.
#![allow(dead_code)]

#[cfg(windows)]
use kuru_platform::fs::{Directory, NameRetention, Privacy};
use kuru_platform::fs::{make_executable, regular_file_info};
use std::{
    fs::{self, File},
    path::Path,
};

pub fn executable(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    make_executable(&File::open(path).unwrap()).unwrap();
}

pub fn identity(path: &Path) -> kuru_platform::fs::FileIdentity {
    regular_file_info(&File::open(path).unwrap())
        .unwrap()
        .identity
}

pub fn symlink(source: impl AsRef<Path>, destination: impl AsRef<Path>) -> std::io::Result<()> {
    #[cfg(unix)]
    return std::os::unix::fs::symlink(source, destination);
    #[cfg(windows)]
    if source.as_ref().is_dir() {
        std::os::windows::fs::symlink_dir(source, destination)
    } else {
        std::os::windows::fs::symlink_file(source, destination)
    }
}

pub fn mode(path: &Path, permissions: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(permissions)).unwrap();
    }
    #[cfg(windows)]
    {
        // Fixtures create new objects with the desired policy instead of
        // treating Windows's READONLY attribute as a Unix permission mask.
        if path.is_dir() {
            assert_eq!(
                fs::read_dir(path).unwrap().count(),
                0,
                "fixture only replaces its empty directory"
            );
            if permissions & 0o077 == 0 {
                fs::remove_dir(path).unwrap();
                Directory::ensure_private(path).unwrap();
            } else {
                assert!(
                    Directory::open(path, Privacy::OwnerOnly, NameRetention::Movable).is_err(),
                    "public fixture unexpectedly has a private ACL"
                );
            }
        } else if permissions & 0o077 == 0 {
            let bytes = fs::read(path).unwrap();
            fs::remove_file(path).unwrap();
            let parent = Directory::open(
                path.parent().unwrap(),
                Privacy::OwnerOnly,
                NameRetention::Movable,
            )
            .unwrap();
            let mut file = parent.create_new(path.file_name().unwrap()).unwrap();
            std::io::Write::write_all(&mut file, &bytes).unwrap();
        } else {
            let public = path
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join(format!("ordinary-{}", uuid::Uuid::new_v4()));
            fs::write(&public, fs::read(path).unwrap()).unwrap();
            assert!(
                kuru_platform::fs::require_private(&File::open(&public).unwrap()).is_err(),
                "public fixture unexpectedly has a private ACL"
            );
            fs::remove_file(path).unwrap();
            fs::rename(public, path).unwrap();
        }
    }
}
