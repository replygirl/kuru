//! The previous published release's own updater installs a new candidate.
//!
//! Shared by the ordinary per-platform CI acceptance and the release-time
//! staged Windows check. Acceptance from the immediately previous release is a
//! floor, not the compatibility policy: updating from any installed release
//! must remain possible and succeed.

use anyhow::{Context, Result, ensure};
use kuru_delivery::{
    archive,
    command::{self, Command},
    published::PreviousRelease,
    shell_support, targets,
};
use std::{ffi::OsString, fs, path::Path, time::Duration};

const DEADLINE: Duration = Duration::from_secs(180);

/// A candidate release directory the previous updater reads through
/// `--release-base`, and what installing it must produce.
pub struct Candidate<'a> {
    /// Local release directory holding `SHA256SUMS`, the core archive and its
    /// paired shell-support envelope.
    pub directory: &'a str,
    /// The version the previous updater is asked to install. Ordinary CI
    /// packages the tree under a synthetic next patch version.
    pub requested_version: &'a str,
    /// The version the installed candidate binary reports, which is the
    /// workspace version it was built from. It can differ from the requested
    /// version, so the exact installed bytes are the discriminating check.
    pub reported_version: &'a str,
    pub executable: &'a [u8],
    pub support: &'a shell_support::Files,
}

/// Fresh user, configuration, cache, data, state and temporary roots for an
/// `env_clear`ed Kuru child. No credential or token is ever passed through.
pub fn isolated_environment(root: &Path, project: &Path) -> Result<Vec<(OsString, OsString)>> {
    #[cfg(windows)]
    {
        kuru_delivery::mise_isolation::prepare(root, project)
    }
    #[cfg(unix)]
    {
        let _ = project;
        let mut environment = Vec::new();
        for (name, relative) in [
            ("HOME", "home"),
            ("XDG_CONFIG_HOME", "xdg-config"),
            ("XDG_CACHE_HOME", "xdg-cache"),
            ("XDG_DATA_HOME", "xdg-data"),
            ("XDG_STATE_HOME", "xdg-state"),
            ("TMPDIR", "tmp"),
        ] {
            let path = root.join(relative);
            fs::create_dir_all(&path)?;
            environment.push((name.into(), path.into()));
        }
        environment.push(("PATH".into(), "/usr/bin:/bin".into()));
        Ok(environment)
    }
}

fn write_executable(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::write(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// Install the authenticated previous release as a real installation would,
/// run its own `kuru update` against the candidate directory, and require the
/// exact candidate executable and shell support afterwards.
pub async fn previous_updater_accepts_candidate(
    previous: &PreviousRelease,
    candidate: &Candidate<'_>,
) -> Result<()> {
    let target = targets::find(previous.target)?;
    ensure!(
        target.triple == archive::host_target()?,
        "previous-updater acceptance must run the native {} release on its own host",
        target.triple
    );
    let old_version = previous.version.as_str();
    let requested = candidate.requested_version;
    let root = tempfile::tempdir()?;
    let old_release_dir = root.path().join("verified-old-release");
    fs::create_dir(&old_release_dir)?;
    // Already authenticated against its own SHA256SUMS and GitHub's digests;
    // the installer path verifies these local copies again before extraction.
    fs::write(old_release_dir.join("SHA256SUMS"), &previous.manifest)?;
    fs::write(
        old_release_dir.join(archive::archive_name(old_version, target.triple)?),
        &previous.archive,
    )?;
    if let Some(envelope) = &previous.support {
        fs::write(
            old_release_dir.join(shell_support::archive_name(old_version, target.triple)?),
            envelope,
        )?;
    }
    let old_directory = old_release_dir
        .to_str()
        .context("isolated old release directory is not Unicode")?;
    let (old_executable, old_support) =
        archive::verified_release(old_directory, old_version, target.triple).await?;

    let installation = root.path().join("old-installed-bin");
    let project = root.path().join("project");
    fs::create_dir(&installation)?;
    fs::create_dir(&project)?;
    let installed = installation.join(target.executable);
    write_executable(&installed, &old_executable)?;
    // Model a real previous installation: a support-aware release installs
    // its own versioned support tree beside the executable, as the installers
    // do, and that tree must survive the update unchanged.
    let old_support_tree = installation
        .join("share")
        .join("kuru")
        .join(old_version)
        .join(target.triple);
    if let Some(files) = &old_support {
        shell_support::install_versioned(files, &installation, old_version, target.triple)?;
        ensure!(
            shell_support::read_generated(&old_support_tree)? == *files,
            "previous v{old_version} support tree was not installed exactly"
        );
        #[cfg(unix)]
        shell_support::publish_stable_man(files, &installation)?;
    }
    let environment = isolated_environment(root.path(), &project)?;
    let command = || {
        let mut command = Command::new(&installed);
        command.env_clear().current_dir(&project);
        for (name, value) in &environment {
            command.env(name, value);
        }
        command
    };
    let mut version_probe = command();
    version_probe.arg("--version");
    let output = command::output(&mut version_probe, DEADLINE).await?;
    ensure!(
        output.status.success()
            && output.stdout.as_slice() == format!("kuru {old_version}\n").as_bytes(),
        "verified old executable did not identify as v{old_version}: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mut update = command();
    update
        .args(["update", "--version", requested, "--release-base"])
        .arg(candidate.directory);
    let output = command::output(&mut update, DEADLINE).await?;
    ensure!(
        output.status.success(),
        "actual v{old_version} updater rejected candidate v{requested}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    println!(
        "previous v{old_version} {} updater installed candidate v{requested}: {}",
        target.triple,
        String::from_utf8_lossy(&output.stdout).trim()
    );
    ensure!(
        fs::read(&installed)? == candidate.executable,
        "old updater did not install the exact verified candidate executable"
    );
    // A marked previous core shipped with the support-aware updater, which
    // stages the candidate's versioned snapshot before replacing itself.
    let managed = installation.join("share");
    if let Some(files) = &old_support {
        ensure!(
            shell_support::read_generated(
                &managed.join("kuru").join(requested).join(target.triple)
            )? == *candidate.support,
            "support-aware v{old_version} updater did not install the exact candidate support"
        );
        ensure!(
            shell_support::read_generated(&old_support_tree)? == *files,
            "v{old_version} update changed the previously installed support tree"
        );
        // Unix installations also publish a stable MANPATH page; Windows
        // keeps only the versioned support tree.
        #[cfg(unix)]
        ensure!(
            fs::read(managed.join("man/man1/kuru.1"))?.as_slice()
                == candidate
                    .support
                    .get("man/kuru.1")
                    .context("candidate support has no man page")?,
            "support-aware v{old_version} updater did not publish the candidate's stable man page"
        );
    } else {
        ensure!(
            !managed.exists(),
            "executable-only v{old_version} updater unexpectedly claimed managed shell support"
        );
    }
    let mut new_version = command();
    new_version.arg("--version");
    let output = command::output(&mut new_version, DEADLINE).await?;
    ensure!(
        output.status.success()
            && String::from_utf8_lossy(&output.stdout).trim()
                == format!("kuru {}", candidate.reported_version),
        "upgraded executable did not report its built version {}",
        candidate.reported_version
    );

    // Not a repair of anything broken: this proves the upgraded binary's own
    // regeneration matches the candidate sidecar byte-for-byte, independent of
    // whatever the old updater itself did or did not install as support.
    let regenerated = root.path().join("regenerated-support-sidecar");
    for (name, args) in [
        ("completions/kuru.bash", &["completions", "bash"][..]),
        ("completions/_kuru", &["completions", "zsh"][..]),
        ("completions/kuru.fish", &["completions", "fish"][..]),
        ("completions/kuru.ps1", &["completions", "powershell"][..]),
        ("man/kuru.1", &["man"][..]),
    ] {
        let mut generator = command();
        generator.args(args);
        let output = command::output(&mut generator, DEADLINE).await?;
        ensure!(
            output.status.success()
                && candidate.support.get(name) == Some(output.stdout.as_slice()),
            "upgraded executable did not regenerate exact candidate support {name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let destination = regenerated.join(name);
        fs::create_dir_all(
            destination
                .parent()
                .context("regenerated support member has no parent")?,
        )?;
        fs::write(&destination, &output.stdout)?;
        ensure!(
            fs::read(&destination)? == output.stdout,
            "regeneration matching the candidate sidecar changed on disk for {name}"
        );
    }
    for (name, value) in &environment {
        if name == "XDG_DATA_HOME" || name == "APPDATA" {
            ensure!(
                !Path::new(value).join("kuru").exists(),
                "old update or support sidecar regeneration created application private data"
            );
        }
    }
    root.close()
        .context("retire isolated old-updater acceptance root")
}
