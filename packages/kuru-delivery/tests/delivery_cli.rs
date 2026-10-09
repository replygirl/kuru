#![cfg(feature = "tooling")]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Output,
};

use kuru_delivery::command::BlockingCommand as Command;
use kuru_delivery::{
    archive::{TARGETS, archive_name, digest},
    shell_support,
};
#[path = "support/files.rs"]
mod files;
#[path = "support/launch_budget.rs"]
mod launch_budget;
use files::symlink;

struct Fixture {
    root: tempfile::TempDir,
    binary: PathBuf,
    release: PathBuf,
    support_input: PathBuf,
    destination: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let binary = root.path().join("input binary");
        files::executable(
            &binary,
            &fs::read(env!("CARGO_BIN_EXE_kuru-delivery-fixture")).unwrap(),
        );
        let release = root.path().join("release files");
        let support_input = root.path().join("generated support");
        fs::create_dir_all(support_input.join("completions")).unwrap();
        fs::create_dir_all(support_input.join("man")).unwrap();
        for name in shell_support::NAMES {
            fs::write(support_input.join(name), format!("generated {name}\n")).unwrap();
        }
        let destination = root.path().join("installed bin");
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("kuru"), b"previous executable").unwrap();
        Self {
            root,
            binary,
            release,
            support_input,
            destination,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_kuru-delivery"));
        command
            .current_dir(self.root.path())
            .env_remove("KURU_RELEASE_BASE")
            .env_remove("KURU_INSTALL_DIR")
            .env_remove("KURU_DOCS_BASE");
        command
    }

    fn package(&self) -> Output {
        let core = self
            .command()
            .args(["package", "--binary"])
            .arg(&self.binary)
            .args(["--target", TARGETS[0], "--version", "v0.2.0", "--output"])
            .arg(&self.release)
            .output()
            .unwrap();
        if core.status.success() {
            let support = self
                .command()
                .args(["package-shell-support", "--input"])
                .arg(&self.support_input)
                .args(["--target", TARGETS[0], "--version", "v0.2.0", "--output"])
                .arg(&self.release)
                .output()
                .unwrap();
            assert!(
                support.status.success(),
                "{}",
                String::from_utf8_lossy(&support.stderr)
            );
        }
        core
    }

    fn archive(&self) -> PathBuf {
        self.release
            .join(archive_name("0.2.0", TARGETS[0]).unwrap())
    }

    fn checksums(&self) {
        let core = fs::read_to_string(self.archive().with_extension("gz.sha256")).unwrap();
        let support = shell_support::archive_name("0.2.0", TARGETS[0]).unwrap();
        let sidecar = fs::read_to_string(self.release.join(format!("{support}.sha256"))).unwrap();
        fs::write(self.release.join("SHA256SUMS"), format!("{core}{sidecar}")).unwrap();
    }

    fn install(&self) -> Output {
        self.command()
            .args(["install", "--version", "0.2.0", "--target", TARGETS[0]])
            .env("KURU_RELEASE_BASE", &self.release)
            .env("KURU_INSTALL_DIR", &self.destination)
            .output()
            .unwrap()
    }

    fn unchanged(&self) {
        assert_eq!(
            fs::read(self.destination.join("kuru")).unwrap(),
            b"previous executable"
        );
        assert_eq!(
            fs::read_dir(&self.destination).unwrap().count(),
            1,
            "staging files leaked"
        );
    }
}

fn success(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn failure(output: &Output, expected: &str) {
    assert!(
        !output.status.success(),
        "unexpected success: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains(expected), "{error}");
}

#[test]
fn real_package_and_env_configured_install_run_the_result() {
    let fixture = Fixture::new();
    let packaged = fixture.package();
    assert_eq!(
        fs::canonicalize(success(&packaged).trim()).unwrap(),
        fs::canonicalize(fixture.archive()).unwrap()
    );
    fixture.checksums();
    let manifest = fs::read_to_string(fixture.release.join("SHA256SUMS")).unwrap();
    assert!(manifest.starts_with(&digest(&fs::read(fixture.archive()).unwrap())));
    let installed = success(&fixture.install());
    assert!(installed.contains("Installed Kuru 0.2.0 at"));
    assert!(installed.contains(fixture.destination.to_str().unwrap()));
    let run = Command::new(fixture.destination.join("kuru"))
        .output()
        .unwrap();
    assert_eq!(success(&run), "native fixture 0.2.0\n");
    assert_eq!(
        fs::read_dir(&fixture.destination).unwrap().count(),
        2 + usize::from(cfg!(windows))
    );

    let explicit = fixture.root.path().join("explicit destination");
    let output = fixture
        .command()
        .args([
            "install",
            "--version",
            "0.2.0",
            "--target",
            TARGETS[0],
            "--release-base",
        ])
        .arg(&fixture.release)
        .arg("--install-dir")
        .arg(&explicit)
        .env(
            "KURU_INSTALL_DIR",
            fixture.root.path().join("unused destination"),
        )
        .output()
        .unwrap();
    success(&output);
    assert_eq!(
        fs::read(explicit.join("kuru")).unwrap(),
        fs::read(&fixture.binary).unwrap()
    );
    assert!(!fixture.root.path().join("unused destination").exists());
}

#[test]
fn real_install_rejects_corruption_and_checksum_ambiguity_without_replacement() {
    let fixture = Fixture::new();
    success(&fixture.package());
    fixture.checksums();
    let archive = fs::read(fixture.archive()).unwrap();
    fs::write(fixture.archive(), b"corrupt").unwrap();
    failure(&fixture.install(), "checksum mismatch");
    fixture.unchanged();
    fs::write(fixture.archive(), archive).unwrap();
    let manifest = fixture.release.join("SHA256SUMS");
    fs::write(&manifest, fs::read_to_string(&manifest).unwrap().repeat(2)).unwrap();
    failure(&fixture.install(), "exactly once");
    fixture.unchanged();
}

#[test]
fn real_install_rejects_verified_traversal_and_symlink_destinations() {
    let fixture = Fixture::new();
    fs::create_dir(&fixture.release).unwrap();
    let mut archive = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::default(),
    ));
    let mut header = tar::Header::new_ustar();
    header.as_mut_bytes()[..7].copy_from_slice(b"../kuru");
    header.set_mode(0o755);
    header.set_size(4);
    header.set_entry_type(tar::EntryType::Regular);
    header.set_cksum();
    archive.append(&header, &b"evil"[..]).unwrap();
    let bytes = archive.into_inner().unwrap().finish().unwrap();
    fs::write(fixture.archive(), &bytes).unwrap();
    fs::write(
        fixture.release.join("SHA256SUMS"),
        format!(
            "{}  {}\n",
            digest(&bytes),
            fixture.archive().file_name().unwrap().to_str().unwrap()
        ),
    )
    .unwrap();
    failure(&fixture.install(), "unsafe paths");
    fixture.unchanged();
    assert!(!fixture.root.path().join("kuru").exists());

    success(&fixture.package());
    fixture.checksums();
    let outside = fixture.root.path().join("outside executable");
    fs::write(&outside, b"outside preserved").unwrap();
    fs::remove_file(fixture.destination.join("kuru")).unwrap();
    symlink(&outside, fixture.destination.join("kuru")).unwrap();
    failure(&fixture.install(), "not a symlink");
    assert_eq!(fs::read(outside).unwrap(), b"outside preserved");
    assert!(fixture.destination.join("kuru").is_symlink());
}

#[test]
fn cli_rejects_missing_inputs_invalid_versions_and_unsupported_targets() {
    let fixture = Fixture::new();
    let help = fixture.command().arg("--help").output().unwrap();
    assert!(success(&help).contains("Install a checksum-verified release"));
    let missing = fixture
        .command()
        .args(["install", "--version", "0.2.0"])
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(2));
    failure(&missing, "--release-base");
    for (value, expected) in [
        ("latest", "explicit semantic version"),
        ("../../escape", "explicit semantic version"),
    ] {
        let output = fixture
            .command()
            .args(["install", "--version", value, "--release-base"])
            .arg(&fixture.release)
            .arg("--install-dir")
            .arg(&fixture.destination)
            .output()
            .unwrap();
        failure(&output, expected);
        fixture.unchanged();
    }
    let target = fixture
        .command()
        .args([
            "install",
            "--version",
            "0.2.0",
            "--target",
            "unknown",
            "--release-base",
        ])
        .arg(&fixture.release)
        .arg("--install-dir")
        .arg(&fixture.destination)
        .output()
        .unwrap();
    failure(&target, "unsupported release target");
    let no_home = fixture
        .command()
        .args(["install", "--version", "0.2.0", "--release-base"])
        .arg(&fixture.release)
        .env_remove("HOME")
        .env_remove("LOCALAPPDATA")
        .output()
        .unwrap();
    failure(&no_home, "provide --install-dir");
    fixture.unchanged();
}

fn write(root: &Path, name: &str, content: &str) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

#[test]
fn docs_cli_uses_default_output_path_and_environment_base_with_actionable_failures() {
    let fixture = Fixture::new();
    let site = fixture.root.path().join("apps/kuru-docs/.vitepress/dist");
    write(
        &site,
        "index.html",
        "<h1 id=\"site\">Site</h1><a href=\"/preview/#site\">Home</a>",
    );
    write(
        &site,
        "sitemap.xml",
        "<urlset><url><loc>https://example.invalid/preview/</loc></url></urlset>",
    );
    write(&site, "llms.txt", "Public docs index");
    write(&site, "llms-full.txt", "Public docs contents");
    let output = fixture
        .command()
        .arg("docs")
        .env("KURU_DOCS_BASE", "/preview/")
        .output()
        .unwrap();
    assert!(success(&output).contains("passed (/preview/)"));
    let incorrect = fixture
        .command()
        .args(["docs", "--root"])
        .arg(&site)
        .args(["--base", "/kuru/"])
        .env("KURU_DOCS_BASE", "/preview/")
        .output()
        .unwrap();
    failure(&incorrect, "escapes site base");
    write(&site, "index.html", "<img src=\"missing.png\">");
    let broken = fixture
        .command()
        .arg("docs")
        .env("KURU_DOCS_BASE", "/preview/")
        .output()
        .unwrap();
    failure(&broken, "missing.png");
}

#[test]
fn repo_cli_checks_actual_owned_manifests_and_reports_drift() {
    let fixture = Fixture::new();
    write(
        fixture.root.path(),
        "Cargo.toml",
        "[workspace]\nmembers=['apps/example']\n[workspace.dependencies]\nserde='=1.0.229'\n",
    );
    write(
        fixture.root.path(),
        "mise.toml",
        "monorepo_root=true\n[monorepo]\nconfig_roots=['apps/*','packages/*']\n[tools]\nrust='1.98.1'\n",
    );
    write(
        fixture.root.path(),
        "rust-toolchain.toml",
        "[toolchain]\nchannel='1.98.1'\n",
    );
    write(
        fixture.root.path(),
        "AGENTS.md",
        "# Kuru\nCanonical instructions.\n",
    );
    write(fixture.root.path(), "CLAUDE.md", "@AGENTS.md\n");
    // The repository's actual shared lint configuration.
    write(
        fixture.root.path(),
        "clippy.toml",
        include_str!("../../../clippy.toml"),
    );
    write(
        fixture.root.path(),
        "apps/example/Cargo.toml",
        "[package]\nname='example'\nversion='0.1.0'\n[dependencies]\nserde.workspace=true\n",
    );
    write(
        fixture.root.path(),
        "apps/example/mise.toml",
        "[tasks.test]\nrun='cargo test -p example'\n",
    );
    let output = fixture.command().arg("repo").output().unwrap();
    assert!(success(&output).contains("metadata invariants passed"));
    write(
        fixture.root.path(),
        "rust-toolchain.toml",
        "[toolchain]\nchannel='1.97.0'\n",
    );
    let drift = fixture
        .command()
        .args(["repo", "--root"])
        .arg(fixture.root.path())
        .output()
        .unwrap();
    failure(&drift, "Rust pins differ");
}

#[test]
fn package_cli_rejects_input_output_aliases_without_modifying_the_executable() {
    let fixture = Fixture::new();
    fs::create_dir(&fixture.release).unwrap();
    fs::copy(&fixture.binary, fixture.archive()).unwrap();
    let before = fs::read(fixture.archive()).unwrap();
    let output = fixture
        .command()
        .args(["package", "--binary"])
        .arg(fixture.archive())
        .args(["--target", TARGETS[0], "--version", "0.2.0", "--output"])
        .arg(&fixture.release)
        .output()
        .unwrap();
    failure(&output, "must not replace its input");
    assert_eq!(fs::read(fixture.archive()).unwrap(), before);
}

#[test]
fn published_windows_cli_is_explicit_and_native_only() {
    let fixture = Fixture::new();
    let help = fixture
        .command()
        .args(["verify-published-windows", "--help"])
        .output()
        .unwrap();
    let help = success(&help);
    for option in [
        "--version",
        "--expected-sha",
        "--mise",
        "--manifest",
        "--evidence",
        "--run-url",
        "--target",
    ] {
        assert!(help.contains(option), "missing {option}: {help}");
    }

    #[cfg(not(windows))]
    {
        let output = fixture
            .command()
            .args([
                "verify-published-windows",
                "--version",
                "1.2.3",
                "--expected-sha",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "--mise",
                "/missing/mise",
                "--evidence",
                "receipt.json",
                "--run-url",
                "https://github.com/replygirl/kuru/actions/runs/42",
            ])
            .output()
            .unwrap();
        failure(&output, "requires native Windows");
    }
}

fn release_command(root: &Path) -> kuru_delivery::command::Command {
    let mut command = kuru_delivery::command::Command::new(env!("CARGO_BIN_EXE_kuru-release"));
    command.current_dir(root).env_clear().env("HOME", root);
    for key in [
        "PATH",
        "SystemRoot",
        "WINDIR",
        "TEMP",
        "TMP",
        "LLVM_PROFILE_FILE",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
}

async fn run_release(mut command: kuru_delivery::command::Command) -> Output {
    kuru_delivery::command::bounded_output(
        &mut command,
        launch_budget::until_job_deadline(),
        64 * 1024,
    )
    .await
    .unwrap()
}

fn release_snapshot(directory: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    fs::read_dir(directory)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().into_string().unwrap(),
                fs::read(entry.path()).unwrap(),
            )
        })
        .collect()
}

#[tokio::test]
async fn release_cli_signing_copies_preserve_hardlinked_inputs_and_verify_private_results() {
    let fixture = Fixture::new();
    let original = fs::read(&fixture.binary).unwrap();
    let identity = files::identity(&fixture.binary);
    let link = fixture.root.path().join("Cargo hardlink");
    fs::hard_link(&fixture.binary, &link).unwrap();
    let output = fixture.root.path().join("private signing output");
    let mut prepare = release_command(fixture.root.path());
    prepare
        .args(["signing-prepare", "--binary"])
        .arg(&fixture.binary)
        .args(["--target", "x86_64-unknown-linux-gnu", "--output"])
        .arg(&output);
    let prepared = success(&run_release(prepare).await);
    let copy = output.join("kuru");
    assert_eq!(Path::new(prepared.trim()), fs::canonicalize(&copy).unwrap());
    assert_eq!(fs::read(&copy).unwrap(), original);
    assert_ne!(files::identity(&copy), identity);
    let directory = kuru_platform::fs::Directory::open(
        &output,
        kuru_platform::fs::Privacy::OwnerOnly,
        kuru_platform::fs::NameRetention::Movable,
    )
    .unwrap();
    directory.read(std::ffi::OsStr::new("kuru")).unwrap();
    let mut verify = release_command(fixture.root.path());
    verify
        .args(["signing-verify", "--binary"])
        .arg(&copy)
        .args([
            "--target",
            "x86_64-unknown-linux-gnu",
            "--publisher",
            "unused-linux-publisher",
        ]);
    assert!(success(&run_release(verify).await).is_empty());
    fs::write(&copy, b"the signer modified only its private copy").unwrap();
    let mut retry = release_command(fixture.root.path());
    retry
        .args(["signing-prepare", "--binary"])
        .arg(&fixture.binary)
        .args(["--target", "x86_64-unknown-linux-gnu", "--output"])
        .arg(&output);
    assert!(!run_release(retry).await.status.success());
    assert_eq!(
        fs::read(&copy).unwrap(),
        b"the signer modified only its private copy"
    );
    assert_eq!(fs::read(&fixture.binary).unwrap(), original);
    assert_eq!(fs::read(&link).unwrap(), original);
    assert_eq!(files::identity(&fixture.binary), identity);
    assert_eq!(files::identity(&link), identity);
    assert_eq!(fs::read_dir(output).unwrap().count(), 1);
}

#[tokio::test]
async fn release_cli_invalid_signing_inputs_leave_no_generated_output() {
    let fixture = Fixture::new();
    let output = fixture.root.path().join("invalid signing output");
    let mut invalid = release_command(fixture.root.path());
    invalid
        .args(["signing-prepare", "--binary"])
        .arg(&fixture.binary)
        .args(["--target", "unsupported", "--output"])
        .arg(&output);
    failure(&run_release(invalid).await, "unsupported release target");
    assert!(!output.exists());
    let empty = fixture.root.path().join("empty executable");
    files::executable(&empty, b"");
    let mut invalid = release_command(fixture.root.path());
    invalid
        .args(["signing-prepare", "--binary"])
        .arg(empty)
        .args(["--target", TARGETS[0], "--output"])
        .arg(&output);
    failure(&run_release(invalid).await, "nonempty");
    assert!(!output.exists());
    let linked = fixture.root.path().join("linked executable");
    symlink(&fixture.binary, &linked).unwrap();
    let mut invalid = release_command(fixture.root.path());
    invalid
        .args(["signing-prepare", "--binary"])
        .arg(linked)
        .args(["--target", TARGETS[0], "--output"])
        .arg(&output);
    failure(&run_release(invalid).await, "regular file");
    assert!(!output.exists());
    let mut prepare = release_command(fixture.root.path());
    prepare
        .args(["signing-prepare", "--binary"])
        .arg(&fixture.binary)
        .args(["--target", "aarch64-apple-darwin", "--output"])
        .arg(&output);
    success(&run_release(prepare).await);
    let before = release_snapshot(&output);
    let mut verify = release_command(fixture.root.path());
    verify
        .args(["signing-verify", "--binary"])
        .arg(output.join("kuru"))
        .args([
            "--target",
            "aarch64-apple-darwin",
            "--publisher",
            "bad-team",
        ]);
    failure(&run_release(verify).await, "ten-character Team ID");
    assert_eq!(release_snapshot(&output), before);
}

#[tokio::test]
async fn release_cli_verifies_exact_paired_package_inventory_and_checksums() {
    let fixture = Fixture::new();
    success(&fixture.package());
    let verify = || {
        let mut command = release_command(fixture.root.path());
        command
            .args(["verify-package", "--directory"])
            .arg(&fixture.release)
            .args(["--version", "0.2.0", "--target", TARGETS[0]]);
        command
    };
    let original = release_snapshot(&fixture.release);
    assert_eq!(original.len(), 4);
    assert!(success(&run_release(verify()).await).is_empty());
    assert_eq!(release_snapshot(&fixture.release), original);
    let extra = fixture.release.join("unexpected archive");
    fs::write(&extra, b"unexpected").unwrap();
    failure(&run_release(verify()).await, "inventory differs");
    fs::remove_file(extra).unwrap();
    let support_name = shell_support::archive_name("0.2.0", TARGETS[0]).unwrap();
    let support = fixture.release.join(&support_name);
    fs::remove_file(&support).unwrap();
    failure(&run_release(verify()).await, "inventory differs");
    fs::write(&support, &original[&support_name]).unwrap();
    fs::write(fixture.archive(), b"corrupted core").unwrap();
    failure(&run_release(verify()).await, "checksum differs");
    let core_name = fixture
        .archive()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    fs::write(fixture.archive(), &original[&core_name]).unwrap();
    // A matching checksum does not excuse an invalid physical payload inventory.
    let mut archive = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::default(),
    ));
    let mut header = tar::Header::new_ustar();
    header.set_size(4);
    header.set_mode(0o644);
    header.set_entry_type(tar::EntryType::Regular);
    header.set_cksum();
    archive
        .append_data(&mut header, "unexpected", &b"evil"[..])
        .unwrap();
    let malformed = archive.into_inner().unwrap().finish().unwrap();
    fs::write(fixture.archive(), &malformed).unwrap();
    fs::write(
        fixture.release.join(format!("{core_name}.sha256")),
        format!("{}  {core_name}\n", digest(&malformed)),
    )
    .unwrap();
    let corrupt_snapshot = release_snapshot(&fixture.release);
    failure(&run_release(verify()).await, "unsafe paths");
    assert_eq!(release_snapshot(&fixture.release), corrupt_snapshot);
}

#[tokio::test]
async fn release_cli_generates_exact_read_only_homebrew_formula_and_rejects_invalid_candidates() {
    let fixture = Fixture::new();
    for target in TARGETS {
        kuru_delivery::archive::package(&fixture.binary, target, "0.2.0", &fixture.release)
            .unwrap();
        shell_support::package(&fixture.support_input, target, "0.2.0", &fixture.release).unwrap();
    }
    kuru_delivery::release::assets(&fixture.release, "0.2.0".parse().unwrap()).unwrap();
    let original = release_snapshot(&fixture.release);
    let formula_path = fixture.root.path().join("kuru.rb");
    let generate = |repository: &str, output: &Path| {
        let mut command = release_command(fixture.root.path());
        command
            .args(["homebrew-generate", "--version", "0.2.0", "--directory"])
            .arg(&fixture.release)
            .args(["--repository", repository, "--output"])
            .arg(output);
        command
    };
    assert!(success(&run_release(generate("replygirl/kuru", &formula_path)).await).is_empty());
    let formula = fs::read_to_string(&formula_path).unwrap();
    assert!(formula.contains("version \"0.2.0\""));
    assert_eq!(formula.matches("resource \"shell-support\"").count(), 3);
    assert_eq!(formula.matches("sha256 \"").count(), 6);
    for target in TARGETS
        .into_iter()
        .filter(|target| !target.contains("windows"))
    {
        for name in [
            archive_name("0.2.0", target).unwrap(),
            shell_support::archive_name("0.2.0", target).unwrap(),
        ] {
            assert!(formula.contains(&format!(
                "url \"https://github.com/replygirl/kuru/releases/download/v0.2.0/{name}\""
            )));
            assert!(formula.contains(&format!("sha256 \"{}\"", digest(&original[&name]))));
        }
    }
    assert!(!formula.contains("windows"));
    assert_eq!(release_snapshot(&fixture.release), original);
    failure(
        &run_release(generate("unrelated/repository", &formula_path)).await,
        "assets must come from replygirl/kuru",
    );
    assert_eq!(fs::read_to_string(&formula_path).unwrap(), formula);
    let rejected_output = fixture.root.path().join("rejected.rb");
    let manifest = fixture.release.join("SHA256SUMS");
    fs::write(&manifest, b"malformed checksum manifest\n").unwrap();
    let malformed = release_snapshot(&fixture.release);
    failure(
        &run_release(generate("replygirl/kuru", &rejected_output)).await,
        "manifest differs",
    );
    assert!(!rejected_output.exists());
    assert_eq!(release_snapshot(&fixture.release), malformed);
    fs::write(&manifest, &original["SHA256SUMS"]).unwrap();
    let missing = shell_support::archive_name("0.2.0", TARGETS[0]).unwrap();
    fs::remove_file(fixture.release.join(missing)).unwrap();
    let incomplete = release_snapshot(&fixture.release);
    failure(
        &run_release(generate("replygirl/kuru", &rejected_output)).await,
        "paired shell support archive",
    );
    assert!(!rejected_output.exists());
    assert_eq!(release_snapshot(&fixture.release), incomplete);
    assert_eq!(fs::read_to_string(formula_path).unwrap(), formula);
}
