//! Checked native drain barriers for a conversation driver. Live presence and
//! catalog authority belong to the memory owner, never to lockfile contents.

use std::{ffi::OsString, fs::File, path::Path};

use anyhow::{Context, Result, ensure};
use kuru_platform::fs::Directory;
use sha2::{Digest, Sha256};

use crate::{
    files,
    store::{SessionDriverRefusal, SessionDriverRejected},
};

pub(crate) struct NativeSessionLease {
    session: Option<File>,
    maintenance: Option<File>,
    directory: Directory,
    session_name: OsString,
    maintenance_name: OsString,
}

/// The existing project lock held exclusively for memory maintenance. It
/// remains outside every moved store tree and is never unlinked on release.
pub(crate) struct NativeMaintenanceLease {
    file: Option<File>,
    _directory: Directory,
}

/// A definite refusal because another session owns or is draining the native
/// project barrier. Other lock I/O failures retain their original error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NativeMaintenanceBusy;

impl std::fmt::Display for NativeMaintenanceBusy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a project session is active or draining")
    }
}

impl std::error::Error for NativeMaintenanceBusy {}

fn maintenance_lock_error(error: std::fs::TryLockError) -> anyhow::Error {
    match error {
        std::fs::TryLockError::WouldBlock => NativeMaintenanceBusy.into(),
        std::fs::TryLockError::Error(error) => error.into(),
    }
}

impl NativeMaintenanceLease {
    pub(crate) fn acquire(data: &Path, scope: &str) -> Result<Self> {
        let digest = scope
            .strip_prefix("project/")
            .context("maintenance requires canonical project scope")?;
        ensure!(
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "maintenance requires a lowercase SHA256 project scope"
        );
        let directory = files::ensure_private_directory(&data.join("locks"))?;
        let name = OsString::from(format!("{digest}.lock"));
        let file = directory.lock_file(&name)?;
        file.try_lock()
            .map_err(maintenance_lock_error)
            .context("acquire native project maintenance barrier")?;
        if let Err(error) = directory.verify(&name, &file) {
            files::release_lock(file);
            return Err(error.into());
        }
        Ok(Self {
            file: Some(file),
            _directory: directory,
        })
    }
}

impl Drop for NativeMaintenanceLease {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            files::release_lock(file);
        }
    }
}

impl NativeSessionLease {
    pub(crate) fn acquire(data: &Path, project: &Path, session_id: &str) -> Result<Self> {
        crate::store::session_identity("driver session", session_id, 128)?;
        let project = project
            .canonicalize()
            .context("canonical session driver project")?;
        let project_hash = hex(Sha256::digest(project.as_os_str().as_encoded_bytes()));
        let directory = files::ensure_private_directory(&data.join("locks"))?;
        // This is the exact existing CLI project lock, in shared mode. It
        // excludes maintenance but permits drivers of distinct sessions.
        let maintenance_name = OsString::from(format!("{project_hash}.lock"));
        let maintenance = directory.lock_file(&maintenance_name)?;
        maintenance.try_lock_shared().map_err(|error| {
            anyhow::anyhow!("project maintenance ownership is unavailable: {error}")
        })?;
        if let Err(error) = directory.verify(&maintenance_name, &maintenance) {
            files::release_lock(maintenance);
            return Err(error.into());
        }
        let session_name = OsString::from(format!(
            "session-{project_hash}-{}.lock",
            hex(Sha256::digest(session_id.as_bytes()))
        ));
        let session = match directory.lock_file(&session_name) {
            Ok(file) => file,
            Err(error) => {
                files::release_lock(maintenance);
                return Err(error.into());
            }
        };
        match session.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                files::release_lock(maintenance);
                return Err(SessionDriverRejected(SessionDriverRefusal::Draining {
                    session_id: session_id.into(),
                })
                .into());
            }
            Err(std::fs::TryLockError::Error(error)) => {
                files::release_lock(maintenance);
                return Err(error).context("acquire checked session drain barrier");
            }
        }
        if let Err(error) = directory.verify(&session_name, &session) {
            files::release_lock(session);
            files::release_lock(maintenance);
            return Err(error.into());
        }
        Ok(Self {
            session: Some(session),
            maintenance: Some(maintenance),
            directory,
            session_name,
            maintenance_name,
        })
    }

    pub(crate) fn verify(&self) -> Result<()> {
        self.directory.verify(
            &self.maintenance_name,
            self.maintenance
                .as_ref()
                .context("driver maintenance barrier was released")?,
        )?;
        self.directory.verify(
            &self.session_name,
            self.session
                .as_ref()
                .context("driver session barrier was released")?,
        )?;
        Ok(())
    }
}

impl Drop for NativeSessionLease {
    fn drop(&mut self) {
        if let Some(file) = self.session.take() {
            files::release_lock(file);
        }
        if let Some(file) = self.maintenance.take() {
            files::release_lock(file);
        }
    }
}

fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maintenance_lock_error_distinguishes_contention_from_io() {
        let busy = maintenance_lock_error(std::fs::TryLockError::WouldBlock);
        assert!(busy.is::<NativeMaintenanceBusy>());

        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let io = maintenance_lock_error(std::fs::TryLockError::Error(denied));
        assert_eq!(
            io.downcast_ref::<std::io::Error>()
                .map(std::io::Error::kind),
            Some(std::io::ErrorKind::PermissionDenied)
        );
    }

    #[test]
    fn session_native_barriers_exclude_same_driver_and_maintenance() -> Result<()> {
        let root = crate::test_support::tempdir()?;
        let uncanonical_project = root.path().join("project");
        std::fs::create_dir(&uncanonical_project)?;
        let project = uncanonical_project.canonicalize()?;
        let data = root.path().join("private");
        let first = NativeSessionLease::acquire(&data, &project, "first")?;
        let second = NativeSessionLease::acquire(&data, &project, "second")?;
        let scope = crate::service::canonical_project_scope(&project)?;
        let busy = match NativeMaintenanceLease::acquire(&data, &scope) {
            Ok(_) => anyhow::bail!("maintenance passed while real native sessions were admitted"),
            Err(error) => error,
        };
        assert!(busy.is::<NativeMaintenanceBusy>());
        first.verify()?;
        second.verify()?;
        let error = match NativeSessionLease::acquire(&data, &project, "first") {
            Ok(_) => anyhow::bail!("same session acquired a second native drain barrier"),
            Err(error) => error,
        };
        assert_eq!(
            error.downcast_ref::<SessionDriverRejected>().unwrap().0,
            SessionDriverRefusal::Draining {
                session_id: "first".into()
            }
        );
        let directory = files::directory(&data.join("locks"))?;
        let maintenance = directory.lock_file(&first.maintenance_name)?;
        assert!(matches!(
            maintenance.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
        drop(first);
        assert!(matches!(
            maintenance.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
        let replacement = NativeSessionLease::acquire(&data, &project, "first")?;
        drop(replacement);
        drop(second);
        maintenance
            .try_lock()
            .map_err(|error| anyhow::anyhow!("maintenance barrier stayed held: {error}"))?;
        files::release_lock(maintenance);
        Ok(())
    }
}
