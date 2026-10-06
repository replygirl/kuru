//! Resolved installation ownership for Unix self-update.

use std::{
    ffi::OsString,
    fs::File,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use kuru_platform::fs::{Directory, NameRetention, Privacy};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Manager {
    Mise,
    Homebrew,
}

impl Manager {
    pub fn label(self) -> &'static str {
        match self {
            Self::Mise => "mise",
            Self::Homebrew => "Homebrew",
        }
    }

    pub fn update_hint(self) -> &'static str {
        match self {
            Self::Mise => "mise upgrade github:replygirl/kuru",
            Self::Homebrew => "brew upgrade kuru",
        }
    }
}

/// One-invocation capture, independent of repository configuration.
#[derive(Clone, Debug, Default)]
pub struct OwnershipEnv {
    pub home: Option<PathBuf>,
    pub xdg_data_home: Option<PathBuf>,
    pub mise_data_dir: Option<PathBuf>,
    pub homebrew_cellar: Option<PathBuf>,
    pub homebrew_prefix: Option<PathBuf>,
}

impl OwnershipEnv {
    pub fn capture() -> Self {
        let path = |key| std::env::var_os(key).map(PathBuf::from);
        Self {
            home: path("HOME"),
            xdg_data_home: path("XDG_DATA_HOME"),
            mise_data_dir: path("MISE_DATA_DIR"),
            homebrew_cellar: path("HOMEBREW_CELLAR"),
            homebrew_prefix: path("HOMEBREW_PREFIX"),
        }
    }

    fn roots(&self) -> Vec<(Manager, PathBuf)> {
        let mut roots = vec![
            (Manager::Homebrew, PathBuf::from("/opt/homebrew/Cellar")),
            (Manager::Homebrew, PathBuf::from("/usr/local/Cellar")),
            (
                Manager::Homebrew,
                PathBuf::from("/home/linuxbrew/.linuxbrew/Cellar"),
            ),
        ];
        for (manager, base, suffix) in [
            (Manager::Homebrew, &self.homebrew_cellar, ""),
            (Manager::Homebrew, &self.homebrew_prefix, "Cellar"),
            (Manager::Homebrew, &self.home, ".linuxbrew/Cellar"),
            (Manager::Mise, &self.mise_data_dir, "installs"),
            (Manager::Mise, &self.xdg_data_home, "mise/installs"),
            (Manager::Mise, &self.home, ".local/share/mise/installs"),
        ] {
            if let Some(base) = base
                && checked_root(base)
            {
                roots.push((manager, base.join(suffix)));
            }
        }
        roots
    }
}

fn checked_root(path: &Path) -> bool {
    path.is_absolute()
        && !path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
}

/// Pure component matching; callers supply a resolved executable and roots.
pub fn classify(resolved: &Path, environment: &OwnershipEnv) -> Option<Manager> {
    environment
        .roots()
        .into_iter()
        .find_map(|(manager, root)| resolved.starts_with(root).then_some(manager))
}

/// Read-only resolved ownership for advisory hints; grants no replacement authority.
pub fn resolved_manager(invoked: &Path, environment: &OwnershipEnv) -> Result<Option<Manager>> {
    Ok(classify_filesystem_roots(
        &invoked.canonicalize()?,
        environment.roots(),
    ))
}

/// Held non-manager installation facts. Revalidation is a fresh preflight,
/// not durable mutation authority through a later network/build operation.
pub struct Installation {
    parent: Directory,
    name: OsString,
    file: File,
}

impl Installation {
    pub(crate) fn into_parts(self) -> (Directory, OsString, File) {
        (self.parent, self.name, self.file)
    }

    pub fn parent(&self) -> &Path {
        self.parent.path()
    }

    pub fn revalidate(&self) -> Result<()> {
        self.parent
            .require_owned_replacement(&self.name, &self.file)
            .context("installed executable changed or is not replaceable by the current user")
    }
}

/// Resolve and refuse package ownership before any self-update effect.
pub fn installed(invoked: &Path, environment: &OwnershipEnv) -> Result<Installation> {
    let resolved = invoked
        .canonicalize()
        .context("installed executable could not be resolved")?;
    // Resolve the complete root set, including default Cellars which may be
    // links to another volume. Pure classification needs no filesystem.
    if let Some(manager) = classify_filesystem_roots(&resolved, environment.roots()) {
        bail!(
            "This Kuru is managed by {} ({resolved:?}); update it with '{}'. 'kuru update' does not replace package-manager files.",
            manager.label(),
            manager.update_hint()
        );
    }
    let parent = Directory::open(
        invoked.parent().context("executable has no parent")?,
        Privacy::Inherited,
        NameRetention::Movable,
    )?;
    let name = invoked.file_name().context("executable has no filename")?;
    let file = parent
        .read(name)
        .context("installed executable must be a checked regular single-link file")?;
    let installation = Installation {
        parent,
        name: name.to_owned(),
        file,
    };
    installation.revalidate().with_context(|| {
        format!(
            "The installed executable {invoked:?} is not owned by and replaceable for the current user; reinstall with the method that installed it"
        )
    })?;
    Ok(installation)
}

fn classify_filesystem_roots(resolved: &Path, roots: Vec<(Manager, PathBuf)>) -> Option<Manager> {
    roots.into_iter().find_map(|(manager, root)| {
        let resolved_root = root.canonicalize().unwrap_or(root);
        resolved.starts_with(resolved_root).then_some(manager)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{ffi::OsStr, os::unix::fs::PermissionsExt};

    #[test]
    fn resolved_manager_components_and_overrides_do_not_claim_near_misses() {
        let environment = OwnershipEnv {
            home: Some("/home/fixture".into()),
            xdg_data_home: Some("/xdg".into()),
            mise_data_dir: Some("/managed/mise".into()),
            homebrew_cellar: Some("/managed/cellar".into()),
            homebrew_prefix: Some("/managed/brew".into()),
        };
        for (path, manager) in [
            ("/opt/homebrew/Cellar/kuru/v/bin/kuru", Manager::Homebrew),
            ("/usr/local/Cellar/kuru/v/bin/kuru", Manager::Homebrew),
            (
                "/home/linuxbrew/.linuxbrew/Cellar/kuru/v/bin/kuru",
                Manager::Homebrew,
            ),
            (
                "/home/fixture/.linuxbrew/Cellar/kuru/v/bin/kuru",
                Manager::Homebrew,
            ),
            ("/managed/cellar/kuru/v/bin/kuru", Manager::Homebrew),
            ("/managed/brew/Cellar/kuru/v/bin/kuru", Manager::Homebrew),
            (
                "/managed/mise/installs/github-replygirl-kuru/v/kuru",
                Manager::Mise,
            ),
            (
                "/xdg/mise/installs/github-replygirl-kuru/v/kuru",
                Manager::Mise,
            ),
            (
                "/home/fixture/.local/share/mise/installs/github-replygirl-kuru/v/kuru",
                Manager::Mise,
            ),
        ] {
            assert_eq!(
                classify(Path::new(path), &environment),
                Some(manager),
                "{path}"
            );
        }
        for path in [
            "/usr/local/Cellar-notes/kuru",
            "/managed/mise/installs-extra/kuru",
            "/xdg/mise-installs/kuru",
            "/home/fixture/bin/kuru",
        ] {
            assert_eq!(classify(Path::new(path), &environment), None, "{path}");
        }
        let relative = OwnershipEnv {
            mise_data_dir: Some("relative".into()),
            ..Default::default()
        };
        assert_eq!(
            classify(Path::new("/fixture/relative/installs/kuru"), &relative),
            None
        );
        assert_eq!(
            Manager::Mise.update_hint(),
            "mise upgrade github:replygirl/kuru"
        );
        assert_eq!(Manager::Homebrew.update_hint(), "brew upgrade kuru");

        // The filesystem capture uses this same complete root list for
        // defaults and overrides; no real package-manager tree is modified.
        let root = tempfile::tempdir().unwrap();
        let actual = root.path().join("other-volume");
        std::fs::create_dir(&actual).unwrap();
        let alias = root.path().join("Cellar");
        std::os::unix::fs::symlink(&actual, &alias).unwrap();
        assert_eq!(
            classify_filesystem_roots(
                &actual.canonicalize().unwrap().join("kuru/version/bin/kuru"),
                vec![(Manager::Homebrew, alias)],
            ),
            Some(Manager::Homebrew)
        );
    }

    #[test]
    fn checked_installation_accepts_readonly_image_but_rejects_changed_names_and_links() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("kuru");
        std::fs::write(&path, b"held original").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o555)).unwrap();
        let environment = OwnershipEnv::default();
        let installation = installed(&path, &environment).unwrap();
        installation.revalidate().unwrap();
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);

        std::fs::hard_link(&path, root.path().join("extra")).unwrap();
        assert!(installation.revalidate().is_err());
        assert!(installed(&path, &environment).is_err());
        std::fs::remove_file(root.path().join("extra")).unwrap();
        std::fs::rename(&path, root.path().join("displaced")).unwrap();
        std::fs::write(&path, b"different occupant").unwrap();
        assert!(installation.revalidate().is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"different occupant");
        std::os::unix::fs::symlink(&path, root.path().join("alias")).unwrap();
        assert!(installed(&root.path().join("alias"), &environment).is_err());
        let directory =
            Directory::open(root.path(), Privacy::Inherited, NameRetention::Movable).unwrap();
        let held = directory.read(OsStr::new("kuru")).unwrap();
        if kuru_platform::unix::own_uids()[1] != 0 {
            std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
            let refusal = directory.require_owned_replacement(OsStr::new("kuru"), &held);
            std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
            assert!(refusal.is_err());
            assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 3);
        }
        // Inspect, never execute or mutate, a stock image owned by a different
        // user. Root-native jobs cannot establish this non-root refusal.
        let stock = Path::new("/bin/cat").canonicalize().unwrap();
        use std::os::unix::fs::MetadataExt;
        if stock.metadata().unwrap().uid() != kuru_platform::unix::own_uids()[1] {
            let stock_parent = Directory::open(
                stock.parent().unwrap(),
                Privacy::Inherited,
                NameRetention::Movable,
            )
            .unwrap();
            let name = stock.file_name().unwrap();
            let stock_file = stock_parent.read(name).unwrap();
            assert!(
                stock_parent
                    .require_owned_replacement(name, &stock_file)
                    .is_err()
            );
        }
    }
}
