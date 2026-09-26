//! Ordinary CI acceptance on every supported platform: the previous published
//! release's own updater installs this tree's release build on the native host.
//!
//! Run through `mise run //packages/kuru-delivery:test:previous-release-update`
//! with `KURU_UPDATE_CANDIDATE_BINARY` naming the absolute release-profile
//! `kuru` executable built from this tree. `GITHUB_TOKEN` is optional and only
//! authenticates the single GitHub release-listing request. Keep this binary
//! single-threaded: the helper writes an executable and then runs it, and a
//! concurrent spawn could inherit that descriptor (Linux ETXTBSY).

#[path = "support/previous_updater.rs"]
mod previous_updater;

use anyhow::{Context, Result, ensure};
use kuru_delivery::{
    archive,
    command::{self, Command},
    published, release, shell_support,
};
use std::{fs, path::PathBuf, time::Duration};

const WORKSPACE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Main's workspace version equals its latest published tag, so the candidate
/// is packaged as the next patch release. The updater binds the archive name
/// and checksums to this requested version, not to what the binary reports.
fn synthetic_candidate_version() -> Result<String> {
    let release::Version([major, minor, patch]) = WORKSPACE_VERSION.parse()?;
    let patch = patch.checked_add(1).context("patch version overflows")?;
    Ok(release::Version([major, minor, patch]).to_string())
}

#[tokio::test]
#[ignore = "requires KURU_UPDATE_CANDIDATE_BINARY and public GitHub release access"]
async fn previous_published_release_updates_to_this_tree() {
    run().await.unwrap();
}

async fn run() -> Result<()> {
    let binary = PathBuf::from(
        std::env::var_os("KURU_UPDATE_CANDIDATE_BINARY")
            .context("KURU_UPDATE_CANDIDATE_BINARY is required")?,
    );
    ensure!(
        binary.is_absolute() && binary.is_file(),
        "KURU_UPDATE_CANDIDATE_BINARY must name an absolute release build of this tree"
    );
    let target = archive::host_target()?;
    let version = synthetic_candidate_version()?;
    let root = tempfile::tempdir()?;
    let releases = root.path().join("candidate-release");
    fs::create_dir(&releases)?;
    let archive_path = archive::package(&binary, target, &version, &releases)?;

    // Generate the paired support from this exact executable in an isolated,
    // token-free environment, as release assembly does from each target build.
    let generator_root = root.path().join("generator");
    let generator_project = generator_root.join("project");
    fs::create_dir_all(&generator_project)?;
    let environment = previous_updater::isolated_environment(&generator_root, &generator_project)?;
    let generated = root.path().join("generated-shell-support");
    for (name, args) in [
        ("completions/kuru.bash", &["completions", "bash"][..]),
        ("completions/_kuru", &["completions", "zsh"][..]),
        ("completions/kuru.fish", &["completions", "fish"][..]),
        ("completions/kuru.ps1", &["completions", "powershell"][..]),
        ("man/kuru.1", &["man"][..]),
    ] {
        let path = generated.join(name);
        fs::create_dir_all(path.parent().context("support file has no parent")?)?;
        let mut generator = Command::new(&binary);
        generator
            .env_clear()
            .current_dir(&generator_project)
            .args(args);
        for (key, value) in &environment {
            generator.env(key, value);
        }
        let output = command::output(&mut generator, Duration::from_secs(180)).await?;
        ensure!(
            output.status.success(),
            "candidate binary could not generate {name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        fs::write(path, output.stdout)?;
    }
    let support_path = shell_support::package(&generated, target, &version, &releases)?;
    let mut manifest = String::new();
    for path in [&archive_path, &support_path] {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .context("packaged asset name is not Unicode")?;
        manifest.push_str(&fs::read_to_string(
            releases.join(format!("{name}.sha256")),
        )?);
    }
    fs::write(releases.join("SHA256SUMS"), manifest)?;

    let directory = releases
        .to_str()
        .context("candidate release directory is not Unicode")?;
    let (executable, support) = archive::verified_release(directory, &version, target).await?;
    ensure!(
        executable == fs::read(&binary)?,
        "packaged candidate differs from KURU_UPDATE_CANDIDATE_BINARY"
    );
    let support = support.context("packaged candidate lacks its shell support marker")?;
    ensure!(
        support == shell_support::read_generated(&generated)?,
        "packaged candidate support differs from its generated files"
    );

    let token = published::checked_token(std::env::var_os("GITHUB_TOKEN"))?;
    let previous = published::previous_release(&version, target, token.as_deref()).await?;
    println!(
        "candidate v{version} (built as v{WORKSPACE_VERSION}) for {target}; previous release v{} archive_sha256={} support={}",
        previous.version,
        archive::digest(&previous.archive),
        if previous.support.is_some() {
            "published"
        } else {
            "absent"
        }
    );
    previous_updater::previous_updater_accepts_candidate(
        &previous,
        &previous_updater::Candidate {
            directory,
            requested_version: &version,
            reported_version: WORKSPACE_VERSION,
            executable: &executable,
            support: &support,
        },
    )
    .await?;
    root.close()
        .context("retire isolated candidate release root")
}
