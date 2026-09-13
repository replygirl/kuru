//! Minimal native mise environment shared by published and candidate acceptance.

use anyhow::Result;
#[cfg(windows)]
use std::fs;
use std::{ffi::OsString, path::Path};

/// Create empty user, configuration, cache and state roots and return the
/// complete environment for an `env_clear`ed native mise child.
pub fn prepare(root: &Path, project: &Path) -> Result<Vec<(OsString, OsString)>> {
    #[cfg(not(windows))]
    {
        let _ = (root, project);
        anyhow::bail!("published mise isolation is only supported on native Windows")
    }
    #[cfg(windows)]
    {
        use kuru_platform::windows::process::system_directory;

        let mut environment = Vec::new();
        for (name, relative) in [
            ("HOME", "home"),
            ("USERPROFILE", "home"),
            ("APPDATA", "appdata"),
            ("LOCALAPPDATA", "local"),
            ("TMP", "tmp"),
            ("TEMP", "tmp"),
            ("MISE_CONFIG_DIR", "mise-config"),
            ("MISE_DATA_DIR", "mise-data"),
            ("MISE_CACHE_DIR", "mise-cache"),
            ("MISE_STATE_DIR", "mise-state"),
            ("MISE_TMP_DIR", "mise-tmp"),
            ("MISE_SYSTEM_CONFIG_DIR", "system-config"),
            ("MISE_SYSTEM_DATA_DIR", "system-data"),
            ("GH_CONFIG_DIR", "gh"),
            ("XDG_CONFIG_HOME", "appdata"),
            ("XDG_CACHE_HOME", "xdg-cache"),
            ("XDG_DATA_HOME", "xdg-data"),
            ("XDG_STATE_HOME", "xdg-state"),
        ] {
            let path = root.join(relative);
            fs::create_dir_all(&path)?;
            environment.push((name.into(), path.into()));
        }
        for (name, relative) in [
            ("MISE_GLOBAL_CONFIG_FILE", "global.toml"),
            ("MISE_SYSTEM_CONFIG_FILE", "system.toml"),
        ] {
            let path = root.join(relative);
            fs::write(&path, b"")?;
            environment.push((name.into(), path.into()));
        }
        let system = system_directory()?;
        environment.push((
            "SystemRoot".into(),
            system
                .parent()
                .ok_or_else(|| anyhow::anyhow!("system directory has no parent"))?
                .as_os_str()
                .to_owned(),
        ));
        environment.push(("PATH".into(), system.into()));
        environment.push(("PATHEXT".into(), ".COM;.EXE;.BAT;.CMD".into()));
        environment.push(("PROCESSOR_ARCHITECTURE".into(), "AMD64".into()));
        environment.push(("MISE_CEILING_PATHS".into(), root.into()));
        environment.push(("MISE_TRUSTED_CONFIG_PATHS".into(), project.into()));
        environment.push(("MISE_YES".into(), "1".into()));
        environment.push(("MISE_COLOR".into(), "0".into()));
        Ok(environment)
    }
}
