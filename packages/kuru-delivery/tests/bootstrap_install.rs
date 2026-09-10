#![cfg(unix)]

use std::{
    fs::{self, File},
    io::{self, Read},
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::{Output, Stdio},
    time::Duration,
};

use flate2::{Compression, write::GzEncoder};
use kuru_delivery::archive::{TARGETS, archive_name, digest, package};
use tokio::process::Command;

const LIMIT: u64 = 128 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);
const PREVIOUS: &[u8] = b"previous executable";
const CANDIDATE: &[u8] = b"#!/bin/sh\nprintf executed > \"$KURU_EXECUTION_MARKER\"\n";

struct Fixture {
    root: tempfile::TempDir,
    release: PathBuf,
    destination: PathBuf,
    tools: PathBuf,
    version: &'static str,
    target: &'static str,
}

impl Fixture {
    fn new(target: &'static str, version: &'static str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let release = root.path().join("release files");
        let destination = root.path().join("installed bin");
        let tools = root.path().join("runtime tools");
        fs::create_dir(&destination).unwrap();
        fs::create_dir(&tools).unwrap();
        fs::write(destination.join("kuru"), PREVIOUS).unwrap();
        for name in [
            "bash", "mkdir", "mktemp", "mkfifo", "head", "wc", "cat", "gzip", "tar", "chmod", "mv",
            "rm", "tr", "cmp", "sort",
        ] {
            symlink(system_tool(name), tools.join(name)).unwrap();
        }
        // Use coreutils on Linux and the standard macOS fallback without
        // requiring a second checksum implementation on either host.
        let hash_tool =
            if Path::new("/usr/bin/sha256sum").is_file() || Path::new("/bin/sha256sum").is_file() {
                "sha256sum"
            } else {
                "shasum"
            };
        // macOS's Perl launcher locates shasum's versioned siblings from its
        // invocation path. A relocated symlink breaks that lookup on macOS 14.
        // The fixed system-tool paths contain no shell metacharacters.
        executable(
            &tools.join(hash_tool),
            format!(
                "#!/bin/bash\nexec '{}' \"$@\"\n",
                system_tool(hash_tool).display()
            )
            .as_bytes(),
        );
        executable(
            &tools.join("uname"),
            b"#!/bin/bash\ncase $1 in -s) printf '%s\\n' \"$FIXTURE_OS\";; -m) printf '%s\\n' \"$FIXTURE_ARCH\";; *) exit 90;; esac\n",
        );
        executable(&tools.join("curl"), CURL_FIXTURE.as_bytes());
        let binary = root.path().join("input binary");
        executable(&binary, CANDIDATE);
        let archive = package(&binary, target, version, &release).unwrap();
        fs::copy(
            archive.with_extension("gz.sha256"),
            release.join("SHA256SUMS"),
        )
        .unwrap();
        fs::copy(
            release.join("SHA256SUMS"),
            root.path().join("latest-manifest"),
        )
        .unwrap();
        Self {
            root,
            release,
            destination,
            tools,
            version,
            target,
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }

    fn archive(&self) -> PathBuf {
        self.release
            .join(archive_name(self.version, self.target).unwrap())
    }

    fn checksums(&self) {
        fs::write(
            self.release.join("SHA256SUMS"),
            format!(
                "{}  {}\n",
                digest(&fs::read(self.archive()).unwrap()),
                self.archive().file_name().unwrap().to_str().unwrap()
            ),
        )
        .unwrap();
    }

    fn command(&self) -> Command {
        let mut command = Command::new("/bin/bash");
        command
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("support/install.sh"))
            .current_dir(self.root.path())
            .env_clear()
            .env("PATH", &self.tools)
            .env("HOME", self.path("isolated home"))
            .env("KURU_RELEASE_BASE", &self.release)
            .env("KURU_INSTALL_DIR", &self.destination)
            .env("KURU_EXECUTION_MARKER", self.path("executed"))
            .env("FIXTURE_ROOT", self.root.path())
            .env("FIXTURE_VERSION", self.version)
            .env("FIXTURE_OS", "Darwin")
            .env("FIXTURE_ARCH", "arm64")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true);
        command
    }

    fn explicit(&self) -> Command {
        let mut command = self.command();
        command.args(["--version", self.version, "--target", self.target]);
        command
    }

    async fn run(&self, mut command: Command) -> Output {
        let child = command.spawn().unwrap();
        let mut group = ProcessGroup(Some(child.id().unwrap()));
        let output = tokio::time::timeout(TIMEOUT, child.wait_with_output())
            .await
            .expect("bootstrap timed out; process group cleanup follows")
            .unwrap();
        group.assert_finished();
        output
    }

    fn unchanged(&self) {
        assert_eq!(fs::read(self.destination.join("kuru")).unwrap(), PREVIOUS);
        assert_eq!(
            fs::read_dir(&self.destination).unwrap().count(),
            1,
            "staging leaked"
        );
        assert!(
            !self.path("executed").exists(),
            "bootstrap executed the candidate"
        );
    }

    fn installed(&self, directory: &Path) {
        assert_eq!(fs::read(directory.join("kuru")).unwrap(), CANDIDATE);
        assert_eq!(
            fs::metadata(directory.join("kuru"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
        assert_eq!(
            fs::read_dir(directory).unwrap().count(),
            1,
            "staging leaked"
        );
        assert!(
            !self.path("executed").exists(),
            "bootstrap executed the candidate"
        );
        assert!(!self.tools.join("cargo").exists());
        assert!(!self.tools.join("rustc").exists());
    }

    fn write_archive(&self, entries: &[(&str, &[u8], u32, tar::EntryType)]) {
        let mut archive = tar::Builder::new(GzEncoder::new(
            File::create(self.archive()).unwrap(),
            Compression::fast(),
        ));
        for (name, bytes, mode, kind) in entries {
            let mut header = tar::Header::new_ustar();
            // Raw names allow adversarial traversal fixtures without path sanitization.
            header.as_mut_bytes()[..name.len()].copy_from_slice(name.as_bytes());
            header.set_size(bytes.len() as u64);
            header.set_mode(*mode);
            header.set_uid(0);
            header.set_gid(0);
            header.set_mtime(0);
            header.set_entry_type(*kind);
            if kind.is_hard_link() || kind.is_symlink() {
                header.set_link_name("LICENSE").unwrap();
            }
            header.set_cksum();
            archive.append(&header, *bytes).unwrap();
        }
        archive.into_inner().unwrap().finish().unwrap();
        self.checksums();
    }

    fn sparse(&self, logical_size: u64) {
        let mut archive = tar::Builder::new(GzEncoder::new(
            File::create(self.archive()).unwrap(),
            Compression::fast(),
        ));
        let mut header = tar::Header::new_gnu();
        header.set_path("kuru").unwrap();
        header.set_size(1);
        header.set_mode(0o755);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_entry_type(tar::EntryType::GNUSparse);
        let gnu = header.as_gnu_mut().unwrap();
        gnu.set_real_size(logical_size);
        gnu.sparse[0].set_offset(logical_size - 1);
        gnu.sparse[0].set_length(1);
        header.set_cksum();
        archive.append(&header, &b"x"[..]).unwrap();
        for name in ["LICENSE", "README.md"] {
            let mut header = tar::Header::new_ustar();
            header.set_path(name).unwrap();
            header.set_size(1);
            header.set_mode(0o644);
            header.set_cksum();
            archive.append(&header, &b"d"[..]).unwrap();
        }
        archive.into_inner().unwrap().finish().unwrap();
        self.checksums();
    }
}

fn system_tool(name: &str) -> PathBuf {
    ["/usr/bin", "/bin"]
        .iter()
        .map(|directory| Path::new(directory).join(name))
        .find(|path| path.is_file())
        .unwrap_or_else(|| panic!("system tool {name} is required for bootstrap fixtures"))
}

fn executable(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

// Kill the entire fixture-owned group if a timeout or assertion interrupts a test.
// kill_on_drop alone would leave the Bash installer’s download children behind.
struct ProcessGroup(Option<u32>);

impl ProcessGroup {
    fn assert_finished(&mut self) {
        let pid = self.0.unwrap();
        assert!(
            !signal("-0", &format!("-{pid}")),
            "bootstrap left a live child in group {pid}"
        );
        self.0 = None;
    }
}

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        if let Some(pid) = self.0 {
            signal("-KILL", &format!("-{pid}"));
        }
    }
}

fn signal(kind: &str, pid: &str) -> bool {
    std::process::Command::new("/bin/bash")
        .args(["-c", "kill \"$1\" -- \"$2\"", "fixture", kind, pid])
        .env_clear()
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success()
}

fn success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn failure(output: &Output, expected: &str) {
    assert!(
        !output.status.success(),
        "unexpected success: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains(expected),
        "expected {expected:?}, got {error}"
    );
}

const CURL_FIXTURE: &str = r#"#!/bin/bash
set -euo pipefail
url=${!#}
printf '%s\n' "$url" >> "$FIXTURE_ROOT/requests"
printf '<%s>\n' "$@" >> "$FIXTURE_ROOT/curl-arguments"
[[ $1 == -q ]] || exit 91
case "$url" in
  https://github.com/replygirl/kuru/releases/latest/download/SHA256SUMS)
    asset="$FIXTURE_ROOT/latest-manifest" ;;
  https://github.com/replygirl/kuru/releases/download/v"$FIXTURE_VERSION"/*|https://fixture.invalid/releases/"$FIXTURE_VERSION"/*)
    asset="$FIXTURE_ROOT/release files/${url##*/}" ;;
  *) printf 'unexpected fixture URL: %s\n' "$url" >&2; exit 92 ;;
esac
case ${FIXTURE_TRANSPORT:-ok} in
  block)
    printf '%s\n' "$$" > "$FIXTURE_ROOT/producer-ready"
    exec /bin/sleep 60 ;;
  overflow-hold)
    printf '%s\n' "$$" > "$FIXTURE_ROOT/producer-ready"
    printf '%65537s' ''
    exec /bin/sleep 60 ;;
  fail-after-data)
    cat "$asset"
    exit 22 ;;
esac
exec cat "$asset"
"#;

#[tokio::test]
async fn actual_package_installs_with_system_bash_and_no_compiler_or_candidate_execution() {
    let fixture = Fixture::new(TARGETS[0], "0.2.0");
    let output = fixture.run(fixture.explicit()).await;
    success(&output);
    fixture.installed(&fixture.destination);
    assert!(String::from_utf8_lossy(&output.stdout).contains("Installed Kuru 0.2.0 at"));

    let explicit = fixture.path("explicit destination");
    let mut command = fixture.command();
    command
        .args(["--version=v0.2.0", "--target", TARGETS[0], "--release-base"])
        .arg(&fixture.release)
        .arg("--install-dir")
        .arg(&explicit)
        .env("KURU_RELEASE_BASE", "http://must-not-be-used.invalid");
    success(&fixture.run(command).await);
    fixture.installed(&explicit);
}

#[tokio::test]
async fn latest_is_resolved_once_then_downloads_the_selected_immutable_version() {
    let fixture = Fixture::new(TARGETS[0], "0.2.0-rc.1");
    let mut command = fixture.command();
    command.env_remove("KURU_RELEASE_BASE");
    success(&fixture.run(command).await);
    fixture.installed(&fixture.destination);
    assert_eq!(
        fs::read_to_string(fixture.path("requests"))
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        [
            "https://github.com/replygirl/kuru/releases/latest/download/SHA256SUMS",
            "https://github.com/replygirl/kuru/releases/download/v0.2.0-rc.1/kuru-0.2.0-rc.1-aarch64-apple-darwin.tar.gz",
        ]
    );
    let arguments = fs::read_to_string(fixture.path("curl-arguments")).unwrap();
    for required in [
        "<--proto>\n<=https>",
        "<--proto-redir>\n<=https>",
        "<--max-time>\n<60>",
        "<--globoff>",
        "<--fail>",
    ] {
        assert!(
            arguments.contains(required),
            "missing transport constraint: {required}"
        );
    }
}

#[tokio::test]
async fn explicit_github_and_custom_mirror_versions_use_literal_release_directories() {
    for custom in [false, true] {
        let fixture = Fixture::new(TARGETS[0], "0.2.0");
        let mut command = fixture.explicit();
        command.env_remove("KURU_RELEASE_BASE");
        if custom {
            command.args(["--release-base", "https://fixture.invalid/releases/0.2.0/"]);
        }
        success(&fixture.run(command).await);
        fixture.installed(&fixture.destination);
        let requests = fs::read_to_string(fixture.path("requests")).unwrap();
        assert!(!requests.contains("latest"));
        assert_eq!(requests.lines().count(), 2);
        assert!(requests.lines().next().unwrap().ends_with("/SHA256SUMS"));
    }
}

#[tokio::test]
async fn host_detection_selects_each_supported_archive_and_rejects_unknown_hosts() {
    for (os, arch, target) in [
        ("Darwin", "arm64", TARGETS[0]),
        ("Darwin", "x86_64", TARGETS[1]),
        ("Linux", "aarch64", TARGETS[2]),
        ("Linux", "x86_64", TARGETS[3]),
    ] {
        let fixture = Fixture::new(target, "0.2.0");
        let mut command = fixture.command();
        command
            .args(["--version", "0.2.0"])
            .env("FIXTURE_OS", os)
            .env("FIXTURE_ARCH", arch);
        success(&fixture.run(command).await);
        fixture.installed(&fixture.destination);
    }
    let fixture = Fixture::new(TARGETS[0], "0.2.0");
    let mut command = fixture.command();
    command
        .args(["--version", "0.2.0"])
        .env("FIXTURE_OS", "FreeBSD");
    failure(&fixture.run(command).await, "unsupported platform");
    fixture.unchanged();
}

#[tokio::test]
async fn argument_and_source_validation_precedes_download_or_replacement() {
    let fixture = Fixture::new(TARGETS[0], "0.2.0");
    for (args, error) in [
        (vec![], "custom release base requires --version"),
        (vec!["--unknown"], "unknown option"),
        (vec!["--version"], "requires a value"),
        (
            vec!["--version", "../../escape"],
            "explicit semantic version",
        ),
        (vec!["--version", "latest"], "explicit semantic version"),
        (
            vec!["--version=0.2.0", "--target=unknown"],
            "unsupported platform",
        ),
    ] {
        let mut command = fixture.command();
        command.args(args);
        failure(&fixture.run(command).await, error);
        fixture.unchanged();
    }
    for base in [
        "http://fixture.invalid",
        "https://u:p@fixture.invalid",
        "https://fixture.invalid/?query",
        "https://fixture.invalid/#fragment",
        "file:///tmp",
        "https:///missing-host",
    ] {
        let mut command = fixture.explicit();
        command.args(["--release-base", base]);
        failure(&fixture.run(command).await, "release base must be HTTPS");
        fixture.unchanged();
    }
    let mut command = fixture.explicit();
    command.env_remove("HOME").env_remove("KURU_INSTALL_DIR");
    failure(&fixture.run(command).await, "provide --install-dir");
    assert!(!fixture.path("requests").exists());
    let mut command = fixture.command();
    command.arg("--help");
    success(&fixture.run(command).await);
}

#[tokio::test]
async fn checksum_missing_corrupt_ambiguous_and_malformed_inputs_preserve_the_installation() {
    let fixture = Fixture::new(TARGETS[0], "0.2.0");
    let manifest = fixture.release.join("SHA256SUMS");
    let original = fs::read(&manifest).unwrap();
    for (data, error) in [
        (original.repeat(2), "exactly once"),
        (b"malformed\n".to_vec(), "malformed checksum"),
        ([original.as_slice(), b"\0"].concat(), "NUL bytes"),
        (Vec::new(), "exactly once"),
    ] {
        fs::write(&manifest, data).unwrap();
        failure(&fixture.run(fixture.explicit()).await, error);
        fixture.unchanged();
    }
    fs::write(&manifest, original).unwrap();
    fs::write(fixture.archive(), b"corrupt").unwrap();
    failure(&fixture.run(fixture.explicit()).await, "checksum mismatch");
    fixture.unchanged();
    fs::remove_file(fixture.archive()).unwrap();
    failure(
        &fixture.run(fixture.explicit()).await,
        "local release asset failed",
    );
    fixture.unchanged();
}

#[tokio::test]
async fn duplicate_latest_versions_are_ambiguous_and_crlf_uppercase_binary_hashes_work() {
    let fixture = Fixture::new(TARGETS[0], "0.2.0");
    let manifest = fs::read_to_string(fixture.release.join("SHA256SUMS")).unwrap();
    fs::write(
        fixture.path("latest-manifest"),
        format!("{manifest}{}", manifest.replace("0.2.0", "0.3.0")),
    )
    .unwrap();
    let mut command = fixture.command();
    command.env_remove("KURU_RELEASE_BASE");
    failure(&fixture.run(command).await, "exactly once");
    assert_eq!(
        fs::read_to_string(fixture.path("requests"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    fixture.unchanged();
    let (hash, name) = manifest.trim_end().split_once("  ").unwrap();
    fs::write(
        fixture.release.join("SHA256SUMS"),
        format!("{} *{name}\r\n", hash.to_ascii_uppercase()),
    )
    .unwrap();
    success(&fixture.run(fixture.explicit()).await);
    fixture.installed(&fixture.destination);
}

#[tokio::test]
async fn unsafe_logical_members_and_nonexecutables_never_extract_other_files() {
    let fixture = Fixture::new(TARGETS[0], "0.2.0");
    fs::write(fixture.path("outside"), b"outside preserved").unwrap();
    for (name, mode, kind, data) in [
        ("../outside", 0o755, tar::EntryType::Regular, CANDIDATE),
        ("kuru", 0o755, tar::EntryType::Symlink, b"".as_slice()),
        ("kuru", 0o755, tar::EntryType::Link, b"".as_slice()),
        ("kuru", 0o644, tar::EntryType::Regular, CANDIDATE),
        ("kuru", 0o755, tar::EntryType::Regular, b"".as_slice()),
    ] {
        fixture.write_archive(&[
            (name, data, mode, kind),
            ("LICENSE", b"license", 0o644, tar::EntryType::Regular),
            ("README.md", b"readme", 0o644, tar::EntryType::Regular),
        ]);
        let output = fixture.run(fixture.explicit()).await;
        failure(&output, "release");
        fixture.unchanged();
        assert_eq!(
            fs::read(fixture.path("outside")).unwrap(),
            b"outside preserved"
        );
        assert!(!fixture.path("LICENSE").exists());
    }
    for names in [
        vec!["kuru", "LICENSE"],
        vec!["kuru", "LICENSE", "README.md", "kuru"],
        vec!["kuru", "LICENSE", "README.md", "extra"],
    ] {
        let entries = names
            .into_iter()
            .map(|name| (name, CANDIDATE, 0o755, tar::EntryType::Regular))
            .collect::<Vec<_>>();
        fixture.write_archive(&entries);
        failure(
            &fixture.run(fixture.explicit()).await,
            "exactly kuru, LICENSE and README.md",
        );
        fixture.unchanged();
    }
}

#[tokio::test]
async fn corrupt_gzip_and_transport_failure_after_valid_bytes_are_not_accepted() {
    let fixture = Fixture::new(TARGETS[0], "0.2.0");
    let mut command = fixture.explicit();
    command
        .env_remove("KURU_RELEASE_BASE")
        .env("FIXTURE_TRANSPORT", "fail-after-data");
    failure(&fixture.run(command).await, "release download failed");
    fixture.unchanged();
    let bytes = fs::read(fixture.archive()).unwrap();
    fs::write(fixture.archive(), &bytes[..bytes.len() - 8]).unwrap();
    fixture.checksums();
    failure(
        &fixture.run(fixture.explicit()).await,
        "expanded release archive failed",
    );
    fixture.unchanged();
}

#[tokio::test]
async fn manifest_and_archive_downloads_are_bounded_including_a_producer_that_stays_open() {
    let fixture = Fixture::new(TARGETS[0], "0.2.0");
    let manifest = fixture.release.join("SHA256SUMS");
    let original = fs::read(&manifest).unwrap();
    File::options()
        .write(true)
        .open(&manifest)
        .unwrap()
        .set_len(64 * 1024 + 1)
        .unwrap();
    failure(
        &fixture.run(fixture.explicit()).await,
        "local release asset exceeds size limit",
    );
    fixture.unchanged();
    fs::write(&manifest, original).unwrap();
    File::options()
        .write(true)
        .open(fixture.archive())
        .unwrap()
        .set_len(LIMIT + 1)
        .unwrap();
    failure(
        &fixture.run(fixture.explicit()).await,
        "local release asset exceeds size limit",
    );
    fixture.unchanged();
    let mut command = fixture.explicit();
    command
        .env_remove("KURU_RELEASE_BASE")
        .env("FIXTURE_TRANSPORT", "overflow-hold");
    failure(
        &fixture.run(command).await,
        "release download exceeds size limit",
    );
    fixture.unchanged();
    let producer = fs::read_to_string(fixture.path("producer-ready")).unwrap();
    assert!(
        !signal("-0", producer.trim()),
        "oversized producer survived cleanup"
    );
}

#[tokio::test]
async fn expansion_limit_includes_non_binary_data_and_sparse_logical_output() {
    let fixture = Fixture::new(TARGETS[0], "0.2.0");
    let mut archive = tar::Builder::new(GzEncoder::new(
        File::create(fixture.archive()).unwrap(),
        Compression::fast(),
    ));
    for (name, size) in [("kuru", 1), ("LICENSE", LIMIT), ("README.md", 1)] {
        let mut header = tar::Header::new_ustar();
        header.set_path(name).unwrap();
        header.set_size(size);
        header.set_mode(0o755);
        header.set_cksum();
        archive.append(&header, io::repeat(0).take(size)).unwrap();
    }
    archive.into_inner().unwrap().finish().unwrap();
    fixture.checksums();
    assert!(fs::metadata(fixture.archive()).unwrap().len() < 1024 * 1024);
    failure(
        &fixture.run(fixture.explicit()).await,
        "expanded release archive exceeds size limit",
    );
    fixture.unchanged();

    // Prove the sparse fixture is valid before changing only its logical size.
    fixture.sparse(1025);
    success(&fixture.run(fixture.explicit()).await);
    let content = fs::read(fixture.destination.join("kuru")).unwrap();
    assert_eq!(content.len(), 1025);
    assert!(content[..1024].iter().all(|byte| *byte == 0));
    assert_eq!(content[1024], b'x');
    fs::write(fixture.destination.join("kuru"), PREVIOUS).unwrap();
    fixture.sparse(LIMIT + 1);
    assert!(fs::metadata(fixture.archive()).unwrap().len() < 4096);
    failure(
        &fixture.run(fixture.explicit()).await,
        "release executable exceeds size limit",
    );
    fixture.unchanged();
}

#[tokio::test]
async fn symlink_and_directory_destinations_are_preserved() {
    let fixture = Fixture::new(TARGETS[0], "0.2.0");
    let destination = fixture.destination.join("kuru");
    let outside = fixture.path("outside");
    fs::write(&outside, PREVIOUS).unwrap();
    fs::remove_file(&destination).unwrap();
    symlink(&outside, &destination).unwrap();
    failure(
        &fixture.run(fixture.explicit()).await,
        "not a symlink or directory",
    );
    assert!(destination.is_symlink());
    assert_eq!(fs::read(&outside).unwrap(), PREVIOUS);
    fs::remove_file(&destination).unwrap();
    fs::create_dir(&destination).unwrap();
    failure(
        &fixture.run(fixture.explicit()).await,
        "not a symlink or directory",
    );
    assert!(destination.is_dir());
    assert_eq!(fs::read_dir(&fixture.destination).unwrap().count(), 1);
}

#[tokio::test]
async fn sigterm_during_download_reaps_both_children_and_preserves_the_previous_binary() {
    let fixture = Fixture::new(TARGETS[0], "0.2.0");
    let mut command = fixture.explicit();
    command
        .env_remove("KURU_RELEASE_BASE")
        .env("FIXTURE_TRANSPORT", "block");
    let child = command.spawn().unwrap();
    let pid = child.id().unwrap();
    let mut group = ProcessGroup(Some(pid));
    let producer = tokio::time::timeout(TIMEOUT, async {
        loop {
            if let Ok(value) = fs::read_to_string(fixture.path("producer-ready"))
                && value.trim().parse::<u32>().is_ok()
            {
                break value;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("fixture never acknowledged the blocked download");
    assert!(signal("-0", producer.trim()));
    assert!(signal("-TERM", &pid.to_string()));
    let output = tokio::time::timeout(Duration::from_secs(5), child.wait_with_output())
        .await
        .expect("SIGTERM did not stop the installer promptly")
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(143),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !signal("-0", producer.trim()),
        "download producer survived SIGTERM"
    );
    group.assert_finished();
    fixture.unchanged();
}
