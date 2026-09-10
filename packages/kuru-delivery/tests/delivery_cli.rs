#![cfg(feature = "tooling")]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Output,
};

use kuru_delivery::archive::{TARGETS, archive_name, digest};
use kuru_delivery::command::BlockingCommand as Command;
#[path = "support/files.rs"]
mod files;
use files::symlink;

struct Fixture {
    root: tempfile::TempDir,
    binary: PathBuf,
    release: PathBuf,
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
        let destination = root.path().join("installed bin");
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("kuru"), b"previous executable").unwrap();
        Self {
            root,
            binary,
            release,
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
        self.command()
            .args(["package", "--binary"])
            .arg(&self.binary)
            .args(["--target", TARGETS[0], "--version", "v0.2.0", "--output"])
            .arg(&self.release)
            .output()
            .unwrap()
    }

    fn archive(&self) -> PathBuf {
        self.release
            .join(archive_name("0.2.0", TARGETS[0]).unwrap())
    }

    fn checksums(&self) {
        fs::copy(
            self.archive().with_extension("gz.sha256"),
            self.release.join("SHA256SUMS"),
        )
        .unwrap();
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
        1 + usize::from(cfg!(windows))
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
