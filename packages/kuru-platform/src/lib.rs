//! Checked native mechanics shared by Kuru's domain packages.
//!
//! Filesystem operations retain object identity and explicit publication outcomes.
//! Windows-only process and IPC operations use owned handles behind safe APIs.
//! Unix built-in shells may use a narrow fresh-process-group owner which keeps
//! standard-child identity through ordered termination, reaping, and absence
//! observation. It is not a general process supervisor or a sandbox.
//! Unix cleanup observes pre-reap membership and failure paths describe trees
//! through bounded, read-only `ps` snapshots. Numeric rows never grant signal
//! authority; repeated cleanup requires fresh retained-root wait ownership.
//! Database, shell, updater and application policy remain with their consumers.

pub mod fs;
pub mod secret;

/// Select the calling build for a native self-launch, never an installation input.
/// On Linux the kernel's executable link remains valid after replacement of
/// the installed pathname; each spawned self selects its own mapped image.
pub fn running_executable() -> std::io::Result<std::path::PathBuf> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = std::path::PathBuf::from("/proc/self/exe");
        let image = std::fs::File::open(&path)?;
        let metadata = image.metadata()?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.permissions().mode() & 0o111 == 0
        {
            return Err(std::io::Error::other(
                "kernel running executable is unavailable",
            ));
        }
        // An unlinked mapped image is legitimate here alone. This does not
        // relax any installed/private/build-input filesystem validation.
        Ok(path)
    }
    #[cfg(not(target_os = "linux"))]
    std::env::current_exe()
}

#[cfg(unix)]
pub mod local_ipc;

#[cfg(unix)]
pub mod unix;

#[cfg(windows)]
#[allow(unsafe_code)]
pub mod windows;

#[cfg(test)]
mod tests {
    #[test]
    fn running_image_remains_a_readable_native_self_launch_input() {
        let image = super::running_executable().unwrap();
        assert!(image.is_absolute());
        let retained = std::fs::File::open(&image).unwrap();
        let metadata = retained.metadata().unwrap();
        assert!(metadata.is_file());
        assert!(metadata.len() > 0);
        #[cfg(target_os = "linux")]
        assert_eq!(image, std::path::Path::new("/proc/self/exe"));
        #[cfg(not(target_os = "linux"))]
        assert_eq!(image, std::env::current_exe().unwrap());
    }
}
