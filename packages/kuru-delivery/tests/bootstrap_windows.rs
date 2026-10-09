#![cfg(all(windows, feature = "tooling"))]

use base64::Engine;
use kuru_archive::zip::{self, Limits, MemberKind, WriteMember};
use kuru_delivery::{
    archive::{MAX_ARCHIVE_BYTES, archive_name, digest, host_target, package},
    command::{Command, output},
    shell_support,
};
use kuru_platform::{
    fs::{
        Directory, NameRetention, Privacy, Publication, regular_file_info, require_private,
        seal_private,
    },
    windows::{
        pipe::Pipe,
        process::{NativeChild, NativeSpawnSpec, Stdio, system_directory},
    },
};
use std::{
    ffi::{OsStr, OsString},
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::Output,
    sync::LazyLock,
    time::Duration,
};
use tokio::io::AsyncReadExt;

const WINDOWS_TARGETS: [&str; 2] = ["x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc"];
// The test executable is native, so its compiled catalog target is the runner's
// native machine target: the one the bootstrap must select.
static TARGET: LazyLock<&'static str> = LazyLock::new(|| {
    let target = host_target().expect("bootstrap tests require a catalog host target");
    assert!(
        WINDOWS_TARGETS.contains(&target),
        "bootstrap tests require a Windows catalog target, found {target}"
    );
    target
});
const VERSION: &str = "0.2.0";

// Stock PowerShell startup and `Add-Type` compilation have no budget of their
// own; `install.ps1` ends its own stalled reads, downloads, DEFLATE framing
// and recovery wait through its exit. See the derivation record.
#[path = "support/launch_budget.rs"]
mod launch_budget;
#[path = "support/powershell_diagnostic.rs"]
mod powershell_diagnostic;
#[path = "support/stock_powershell.rs"]
mod stock_powershell;

struct Fixture {
    root: tempfile::TempDir,
    release: PathBuf,
    install: PathBuf,
    candidate: PathBuf,
    original: Vec<u8>,
    replacement: Vec<u8>,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let release = root.path().join("release files λ");
        let install = root.path().join("ordinary installed bin 日本語");
        fs::create_dir(&install).unwrap();
        let original = fs::read(env!("CARGO_BIN_EXE_kuru-delivery-fixture")).unwrap();
        let mut replacement = original.clone();
        replacement.extend_from_slice(b"KURU_CANDIDATE_MARKER");
        let candidate = root.path().join("candidate.exe");
        fs::write(&candidate, &replacement).unwrap();
        fs::write(install.join("kuru.exe"), &original).unwrap();
        let archive = package(&candidate, *TARGET, VERSION, &release).unwrap();
        let generated = root.path().join("generated support");
        for name in shell_support::NAMES {
            let path = generated.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, format!("generated {name} for {}\n", *TARGET)).unwrap();
        }
        let support = shell_support::package(&generated, *TARGET, VERSION, &release).unwrap();
        fs::write(
            release.join("SHA256SUMS"),
            format!(
                "{}{}",
                fs::read_to_string(archive.with_extension("zip.sha256")).unwrap(),
                fs::read_to_string(support.with_extension("zip.sha256")).unwrap()
            ),
        )
        .unwrap();
        fs::create_dir(root.path().join("temporary compiler output")).unwrap();
        Self {
            root,
            release,
            install,
            candidate,
            original,
            replacement,
        }
    }

    fn environment(&self) -> Vec<(OsString, OsString)> {
        let system = system_directory().unwrap();
        let mut result = vec![
            ("SystemRoot".into(), system.parent().unwrap().into()),
            (
                "PROCESSOR_ARCHITECTURE".into(),
                kuru_delivery::mise_isolation::native_processor_architecture().into(),
            ),
            ("PATH".into(), "".into()),
            (
                "USERPROFILE".into(),
                self.root.path().join("isolated home").into(),
            ),
            (
                "LOCALAPPDATA".into(),
                self.root.path().join("local app data").into(),
            ),
            (
                "TMP".into(),
                self.root.path().join("temporary compiler output").into(),
            ),
            (
                "TEMP".into(),
                self.root.path().join("temporary compiler output").into(),
            ),
            ("KURU_RELEASE_BASE".into(), self.release.clone().into()),
            ("KURU_INSTALL_DIR".into(), self.install.clone().into()),
            (
                "KURU_EXECUTION_MARKER".into(),
                self.root.path().join("executed").into(),
            ),
        ];
        if let Some(value) = std::env::var_os("LLVM_PROFILE_FILE") {
            result.push(("LLVM_PROFILE_FILE".into(), value));
        }
        result
    }

    fn powershell(&self) -> PathBuf {
        let executable = system_directory()
            .unwrap()
            .join("WindowsPowerShell/v1.0/powershell.exe");
        assert!(
            executable.is_file(),
            "stock Windows PowerShell 5.1 is mandatory native acceptance"
        );
        executable
    }

    fn arguments(&self) -> Vec<OsString> {
        ["-NoLogo", "-NoProfile", "-NonInteractive", "-File"]
            .map(OsString::from)
            .into_iter()
            .chain([Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("support/install.ps1")
                .into_os_string()])
            .collect()
    }

    fn command(&self) -> Command {
        let mut command = Command::new(self.powershell());
        command
            .args(self.arguments())
            .args(["-Version", VERSION])
            .current_dir(self.root.path())
            .env_clear()
            .envs(self.environment());
        command
    }

    fn script(&self, source: &str) -> Command {
        let encoded: Vec<u8> = source.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let mut command = Command::new(self.powershell());
        command
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-OutputFormat",
                "Text",
                "-EncodedCommand",
            ])
            .arg(base64::engine::general_purpose::STANDARD.encode(encoded))
            .current_dir(self.root.path())
            .env_clear()
            .envs(self.environment())
            .env(
                "KURU_BOOTSTRAP_SCRIPT",
                Path::new(env!("CARGO_MANIFEST_DIR")).join("support/install.ps1"),
            )
            .env("KURU_TEST_CANDIDATE", &self.candidate);
        command
    }

    fn native(&self) -> NativeSpawnSpec {
        let mut spec = NativeSpawnSpec::new(self.powershell(), self.root.path().to_owned());
        spec.args = self.arguments();
        spec.args.extend(["-Version".into(), VERSION.into()]);
        spec.environment = self.environment();
        spec.stdout = Stdio::Pipe;
        spec.stderr = Stdio::Pipe;
        spec
    }

    async fn run(&self, command: &mut Command) -> Output {
        output(command, launch_budget::until_job_deadline())
            .await
            .expect("bounded stock PowerShell bootstrap process failed")
    }

    fn installed(&self, path: &Path) {
        assert_eq!(fs::read(path.join("kuru.exe")).unwrap(), self.replacement);
        let support = path.join(format!("share/kuru/{VERSION}/{}", *TARGET));
        for name in shell_support::NAMES {
            assert_eq!(
                fs::read(support.join(name)).unwrap(),
                format!("generated {name} for {}\n", *TARGET).into_bytes()
            );
        }
        assert!(!self.root.path().join("executed").exists());
        assert_no_stage(path);
        let state = Directory::open(
            &path.join(".kuru-update"),
            Privacy::OwnerOnly,
            NameRetention::Pinned,
        )
        .unwrap();
        require_private(&state.read(OsStr::new("install.lock")).unwrap()).unwrap();
    }

    fn unchanged(&self) {
        assert_eq!(
            fs::read(self.install.join("kuru.exe")).unwrap(),
            self.original
        );
        assert!(!self.root.path().join("executed").exists());
        assert_no_stage(&self.install);
    }

    fn replace_archive(&self, bytes: &[u8]) {
        let name = archive_name(VERSION, *TARGET).unwrap();
        fs::write(self.release.join(&name), bytes).unwrap();
        fs::write(
            self.release.join("SHA256SUMS"),
            format!("{}  {name}\n", digest(bytes)),
        )
        .unwrap();
    }

    fn support_archive(&self) -> PathBuf {
        self.release
            .join(shell_support::archive_name(VERSION, *TARGET).unwrap())
    }

    async fn crash_gap(&self) {
        let mut spec = NativeSpawnSpec::new(self.install.join("kuru.exe"), self.install.clone());
        spec.args = vec![
            "crash-gap".into(),
            self.candidate.clone().into(),
            self.root.path().join("trusted helper cache").into(),
        ];
        spec.environment = self.environment();
        spec.stdin = Stdio::Pipe;
        spec.stdout = Stdio::Pipe;
        spec.stderr = Stdio::Pipe;
        let mut child = spec.spawn().await.unwrap();
        let mut stdout = child.take_stdout().unwrap();
        let mut stderr = child.take_stderr().unwrap();
        let mut stdin = child.take_stdin().unwrap();
        let receipt: serde_json::Value = serde_json::from_str(&line(&mut stdout).await).unwrap();
        assert_eq!(receipt["ready"], "old_moved");
        assert!(child.try_wait().unwrap().is_none());
        assert!(!self.install.join("kuru.exe").exists());
        terminate(&mut child, &mut stdout, &mut stderr).await;
        stdin.close(Duration::from_secs(10)).await.unwrap();
        assert!(!self.install.join("kuru.exe").exists());
    }
}

fn assert_no_stage(path: &Path) {
    assert!(fs::read_dir(path).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".kuru-install-")
    }));
}

fn stderr_message(result: &Output) -> String {
    powershell_diagnostic::message(&result.stderr)
}

fn other_target() -> &'static str {
    WINDOWS_TARGETS
        .into_iter()
        .find(|target| *target != *TARGET)
        .unwrap()
}

fn pe_machine(target: &str) -> (u16, &'static str) {
    match target {
        "x86_64-pc-windows-msvc" => (0x8664, "x64"),
        "aarch64-pc-windows-msvc" => (0xaa64, "ARM64"),
        other => panic!("{other} is not a Windows catalog target"),
    }
}

fn machine_field(image: &[u8]) -> usize {
    let offset = u32::from_le_bytes(image[60..64].try_into().unwrap()) as usize;
    assert_eq!(
        &image[offset..offset + 4],
        b"PE\0\0",
        "fixture is not a PE image"
    );
    offset + 4
}

// Renames the sealed cached helper itself, so its identity, length and digest
// still match the receipt; only its name changes.
fn rename_helper(cache: &Path, from: &OsStr, to: &OsStr) {
    let pinned = Directory::open(cache, Privacy::OwnerOnly, NameRetention::Pinned).unwrap();
    let movable = Directory::open(cache, Privacy::OwnerOnly, NameRetention::Movable).unwrap();
    let file = movable.read(from).unwrap();
    pinned
        .rename_file(&movable, from, &file, to, Publication::New)
        .unwrap();
}

// Publishes receipt bytes the way the updater does: a private draft replaced
// into place, never an in-place write.
fn replace_receipt(install: &Path, bytes: &[u8]) {
    let state = Directory::open(
        &install.join(".kuru-update"),
        Privacy::OwnerOnly,
        NameRetention::Movable,
    )
    .unwrap();
    let draft = OsStr::new("receipt-fixture.json");
    let mut file = state.create_new(draft).unwrap();
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
    state
        .publish_file(
            &state,
            draft,
            &file,
            OsStr::new("receipt.json"),
            Publication::ReplaceRegular,
        )
        .unwrap();
}

fn success(result: &Output) {
    assert!(
        result.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

async fn line(pipe: &mut Pipe) -> String {
    tokio::time::timeout(launch_budget::until_job_deadline(), async {
        let mut bytes = Vec::new();
        loop {
            let byte = pipe.read_u8().await?;
            if byte == b'\n' {
                break;
            }
            if bytes.len() >= 65536 {
                return Err(io::Error::other("native bootstrap line exceeds bounds"));
            }
            bytes.push(byte);
        }
        String::from_utf8(bytes).map_err(io::Error::other)
    })
    .await
    .expect("native fixture did not acknowledge within its bounded deadline")
    .unwrap()
}

async fn terminate(child: &mut NativeChild, stdout: &mut Pipe, stderr: &mut Pipe) {
    child.terminate().unwrap();
    stdout.close(Duration::from_secs(10)).await.unwrap();
    stderr.close(Duration::from_secs(10)).await.unwrap();
    child.wait(Duration::from_secs(10)).await.unwrap();
}

#[tokio::test]
async fn stock_ps51_installs_real_pe_zip_with_empty_path_and_explicit_options_override_environment()
{
    let fixture = Fixture::new();
    let destination = fixture.root.path().join("explicit install δ");
    let mut command = Command::new(fixture.powershell());
    command
        .args(fixture.arguments())
        .current_dir(fixture.root.path())
        .env_clear()
        .envs(fixture.environment());
    let result = fixture
        .run(
            command
                .arg("-Version")
                .arg("v0.2.0")
                .arg("-Target")
                .arg(*TARGET)
                .arg("-InstallDir")
                .arg(&destination)
                .arg("-ReleaseBase")
                .arg(&fixture.release)
                .env("KURU_RELEASE_BASE", "Z:\\missing"),
        )
        .await;
    success(&result);
    fixture.installed(&destination);
    fixture.unchanged();
    // Run only after installation to prove native bytes, never as bootstrap validation.
    let mut command = Command::new(destination.join("kuru.exe"));
    command
        .arg("--version")
        .env_clear()
        .envs(fixture.environment());
    let result = output(&mut command, Duration::from_secs(20)).await.unwrap();
    success(&result);
    assert_eq!(result.stdout, b"native fixture 0.2.0\n");
    assert!(fixture.root.path().join("executed").is_file());
}

#[tokio::test]
async fn default_install_destination_uses_local_app_data_programs_and_recover_needs_no_release() {
    let fixture = Fixture::new();
    let result = fixture
        .run(fixture.command().env_remove("KURU_INSTALL_DIR"))
        .await;
    success(&result);
    let destination = fixture.root.path().join("local app data/Programs/kuru/bin");
    fixture.installed(&destination);
    let result = fixture
        .run(
            fixture
                .command()
                .arg("-Recover")
                .arg("-InstallDir")
                .arg(&destination)
                .env("KURU_RELEASE_BASE", "https://must-not-contact.invalid"),
        )
        .await;
    success(&result);
    fixture.installed(&destination);
}

#[tokio::test]
async fn bootstrap_supplies_its_stock_commands_without_module_auto_discovery() {
    // The fixture's fresh LOCALAPPDATA gives PowerShell a cold module-analysis
    // cache, as on a new profile. With autoloading disabled, any bootstrap
    // command reached through auto-discovery is refused instead of scanning the
    // module path; only the script's exact PSHOME imports can supply them.
    let fixture = Fixture::new();
    let result = fixture
        .run(&mut fixture.script(&format!(
            r#"
$ErrorActionPreference = 'Stop'
$PSModuleAutoLoadingPreference = 'None'
& $env:KURU_BOOTSTRAP_SCRIPT -Version '{VERSION}'
foreach ($name in @('Microsoft.PowerShell.Management', 'Microsoft.PowerShell.Utility')) {{
    $expected = [IO.Path]::Combine($PSHOME, 'Modules', $name, "$name.psd1")
    $loaded = @(Microsoft.PowerShell.Core\Get-Module -Name $name)
    if ($loaded.Count -ne 1 -or -not [String]::Equals($loaded[0].Path, $expected, [StringComparison]::OrdinalIgnoreCase)) {{ throw "bootstrap did not load the exact PSHOME $name manifest" }}
}}
"#
        )))
        .await;
    success(&result);
    fixture.installed(&fixture.install);
}

#[tokio::test]
async fn marked_windows_core_requires_verified_paired_support_before_publication() {
    let fixture = Fixture::new();
    let core = fixture
        .release
        .join(archive_name(VERSION, *TARGET).unwrap());
    fs::write(
        fixture.release.join("SHA256SUMS"),
        format!(
            "{}  {}\n",
            digest(&fs::read(&core).unwrap()),
            core.file_name().unwrap().to_str().unwrap()
        ),
    )
    .unwrap();
    let mut command = fixture.command();
    let result = fixture.run(&mut command).await;
    assert!(
        stderr_message(&result).contains("paired shell-support archive exactly once"),
        "{}",
        stderr_message(&result)
    );
    fixture.unchanged();

    let sidecar = fixture.support_archive();
    fs::write(
        fixture.release.join("SHA256SUMS"),
        format!(
            "{}  {}\n{}  {}\n",
            digest(&fs::read(&core).unwrap()),
            core.file_name().unwrap().to_str().unwrap(),
            digest(&fs::read(&sidecar).unwrap()),
            sidecar.file_name().unwrap().to_str().unwrap()
        ),
    )
    .unwrap();
    fs::write(sidecar, b"corrupt support").unwrap();
    let mut command = fixture.command();
    let result = fixture.run(&mut command).await;
    assert!(
        stderr_message(&result).contains("Shell-support archive checksum mismatch"),
        "{}",
        stderr_message(&result)
    );
    fixture.unchanged();
}

#[tokio::test]
async fn stock_ps51_accepts_a_verified_unmarked_historical_core_without_support() {
    let fixture = Fixture::new();
    let members = [
        WriteMember {
            name: "kuru.exe",
            kind: MemberKind::File,
            bytes: &fixture.replacement,
            executable: true,
        },
        WriteMember {
            name: "LICENSE",
            kind: MemberKind::File,
            bytes: b"historical license",
            executable: false,
        },
        WriteMember {
            name: "README.md",
            kind: MemberKind::File,
            bytes: b"historical release without a support marker",
            executable: false,
        },
    ];
    fixture.replace_archive(
        &zip::write(
            &members,
            Limits {
                max_compressed_bytes: MAX_ARCHIVE_BYTES as u64,
                max_expanded_bytes: MAX_ARCHIVE_BYTES as u64,
                allow_ntfs_timestamps: false,
            },
        )
        .unwrap(),
    );
    let mut command = fixture.command();
    success(&fixture.run(&mut command).await);
    assert_eq!(
        fs::read(fixture.install.join("kuru.exe")).unwrap(),
        fixture.replacement
    );
    assert!(!fixture.install.join("share").exists());
}

#[tokio::test]
async fn retained_v041_v042_powershell_reader_accepts_the_new_three_member_core() {
    // v0.4.1 and v0.4.2 shipped byte-identical install.ps1 readers. Retain
    // their actual script so this native compatibility check needs no checkout.
    const OLD_READER_SHA256: &str =
        "2425cfa02eea191752a7617b3c1cafffdd96f584348a36ea9c9b0e938d97084d";
    let fixture = Fixture::new();
    let old = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/install-v0.4.2.ps1");
    assert_eq!(digest(&fs::read(&old).unwrap()), OLD_READER_SHA256);
    let mut command = Command::new(fixture.powershell());
    command
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-File"])
        .arg(old)
        .args(["-Version", VERSION])
        .current_dir(fixture.root.path())
        .env_clear()
        .envs(fixture.environment());
    if *TARGET != "x86_64-pc-windows-msvc" {
        // The retained readers install x64 only and refuse any host whose
        // PROCESSOR_ARCHITECTURE is not AMD64 before reading a release, and
        // Windows on Arm offers no x64 PowerShell host to run them under
        // emulation. Here they must refuse without touching the installation;
        // the x64 runner proves they accept the three-member core.
        let result = fixture.run(&mut command).await;
        assert!(!result.status.success());
        assert!(
            stderr_message(&result).contains("on Windows x64"),
            "unexpected legacy reader result: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        fixture.unchanged();
        return;
    }
    success(&fixture.run(&mut command).await);
    assert_eq!(
        fs::read(fixture.install.join("kuru.exe")).unwrap(),
        fixture.replacement
    );
    assert!(!fixture.install.join("share").exists());
    assert!(!fixture.root.path().join("executed").exists());
    assert_no_stage(&fixture.install);
}

#[tokio::test]
async fn malformed_manifest_hash_and_zip_fail_before_changing_existing_native_identity() {
    let fixture = Fixture::new();
    // A bootstrap startup failure cannot satisfy any rejection below. Exercise
    // the same process path successfully against a separate destination first.
    let control = fixture.root.path().join("valid control");
    success(
        &fixture
            .run(fixture.command().arg("-InstallDir").arg(&control))
            .await,
    );
    fixture.installed(&control);
    let identity = regular_file_info(&fs::File::open(fixture.install.join("kuru.exe")).unwrap())
        .unwrap()
        .identity;
    let original_manifest = fs::read(fixture.release.join("SHA256SUMS")).unwrap();
    let mut duplicate = original_manifest.clone();
    duplicate.extend_from_slice(&original_manifest);
    // A release that predates the native target lists only other targets.
    let without_native = String::from_utf8(original_manifest.clone())
        .unwrap()
        .replace(*TARGET, other_target())
        .into_bytes();
    let no_native_archive = format!(
        "Release v{VERSION} publishes no {} archive; nothing was installed",
        *TARGET
    );
    for (manifest, diagnostic) in [
        (b"not a checksum\n".to_vec(), "Malformed checksum manifest"),
        (
            duplicate,
            "Checksum manifest must name the release archive exactly once",
        ),
        (without_native, no_native_archive.as_str()),
        (vec![b'x'; 65537], "exceeds"),
        (
            original_manifest
                .iter()
                .enumerate()
                .map(|(index, byte)| if index < 64 { b'0' } else { *byte })
                .collect(),
            "Release archive checksum mismatch",
        ),
    ] {
        fs::write(fixture.release.join("SHA256SUMS"), manifest).unwrap();
        let result = fixture.run(&mut fixture.command()).await;
        assert!(!result.status.success());
        assert!(
            stderr_message(&result).contains(diagnostic),
            "unexpected rejection: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        fixture.unchanged();
    }
    let members = || {
        ["kuru.exe", "LICENSE", "README.md"].map(|name| WriteMember {
            name,
            kind: MemberKind::File,
            bytes: if name == "kuru.exe" {
                &fixture.replacement
            } else {
                b"license text"
            },
            executable: name == "kuru.exe",
        })
    };
    let limits = Limits {
        max_compressed_bytes: MAX_ARCHIVE_BYTES as u64,
        max_expanded_bytes: MAX_ARCHIVE_BYTES as u64,
        allow_ntfs_timestamps: false,
    };
    let valid = zip::write(&members(), limits).unwrap();
    let mut wrong_mode = members();
    wrong_mode[0].executable = false;
    let mut crc = valid.clone();
    crc[14] ^= 1;
    let central = crc
        .windows(4)
        .position(|bytes| bytes == b"PK\x01\x02")
        .unwrap();
    crc[central + 16] ^= 1;
    let mut duplicate = valid.clone();
    // Three central names still exist, but two entries alias one physical local record.
    let second = central + 46 + "kuru.exe".len();
    duplicate[second + 42..second + 46].copy_from_slice(&0u32.to_le_bytes());
    // LICENSE is a tiny one-block deflate stream. Remove BFINAL without
    // changing expanded bytes or CRC: Framework DeflateStream alone can
    // return all expected bytes on this truncated block sequence.
    let mut unfinished = valid.clone();
    let license_offset =
        u32::from_le_bytes(valid[second + 42..second + 46].try_into().unwrap()) as usize;
    let license_payload = license_offset + 30 + "LICENSE".len();
    assert_eq!(
        unfinished[license_payload] & 1,
        1,
        "small fixture must have a single final deflate block"
    );
    unfinished[license_payload] &= !1;
    let mut non_pe = members();
    non_pe[0].bytes = b"downloaded text is never executed";
    for malformed in [
        zip::write(&members()[..2], limits).unwrap(),
        zip::write(&wrong_mode, limits).unwrap(),
        crc,
        duplicate,
        unfinished,
        zip::write(&non_pe, limits).unwrap(),
        [valid.as_slice(), b"trailing"].concat(),
    ] {
        fixture.replace_archive(&malformed);
        let result = fixture.run(&mut fixture.command()).await;
        assert!(!result.status.success());
        assert!(
            String::from_utf8_lossy(&result.stdout).contains("Verifying Kuru release archive."),
            "ZIP validation was never reached: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        fixture.unchanged();
        assert_eq!(
            regular_file_info(&fs::File::open(fixture.install.join("kuru.exe")).unwrap())
                .unwrap()
                .identity,
            identity
        );
    }
}

#[tokio::test]
async fn stock_ps51_enforces_archive_and_decoded_output_limits_before_publication() {
    let fixture = Fixture::new();
    let identity = regular_file_info(&fs::File::open(fixture.install.join("kuru.exe")).unwrap())
        .unwrap()
        .identity;
    let control = fixture.root.path().join("bounded valid control");
    success(
        &fixture
            .run(fixture.command().arg("-InstallDir").arg(&control))
            .await,
    );
    fixture.installed(&control);

    let name = archive_name(VERSION, *TARGET).unwrap();
    let archive = fixture.release.join(&name);
    let valid = fs::read(&archive).unwrap();
    // A real oversized local file exercises bounded Fetch/ReadBytes before
    // hashing or allocating the full archive. No cap is lowered for the test.
    fs::OpenOptions::new()
        .write(true)
        .open(&archive)
        .unwrap()
        .set_len(MAX_ARCHIVE_BYTES as u64 + 1)
        .unwrap();
    let result = fixture.run(&mut fixture.command()).await;
    assert!(!result.status.success());
    assert!(
        stderr_message(&result).contains("file exceeds size limit"),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!String::from_utf8_lossy(&result.stdout).contains("Verifying Kuru release archive."));
    fixture.unchanged();
    assert_eq!(
        regular_file_info(&fs::File::open(fixture.install.join("kuru.exe")).unwrap())
            .unwrap()
            .identity,
        identity
    );

    let limit = u32::try_from(MAX_ARCHIVE_BYTES).unwrap();
    for (sizes, diagnostic) in [
        (
            [Some(limit + 1), None, None],
            "ZIP compression or member size is invalid",
        ),
        (
            [Some(limit / 2), Some(limit / 2), Some(1)],
            "expanded ZIP exceeds limit",
        ),
        (
            [Some(1), None, None],
            "DEFLATE output exceeds declared size",
        ),
    ] {
        let mut bytes = valid.clone();
        let end = bytes.len() - 22;
        let mut central =
            u32::from_le_bytes(bytes[end + 16..end + 20].try_into().unwrap()) as usize;
        for size in sizes {
            assert_eq!(&bytes[central..central + 4], b"PK\x01\x02");
            let local =
                u32::from_le_bytes(bytes[central + 42..central + 46].try_into().unwrap()) as usize;
            if let Some(size) = size {
                // Both physical records agree: rejection must reach the
                // size/output policy rather than the metadata mismatch guard.
                bytes[local + 22..local + 26].copy_from_slice(&size.to_le_bytes());
                bytes[central + 24..central + 28].copy_from_slice(&size.to_le_bytes());
            }
            let name =
                u16::from_le_bytes(bytes[central + 28..central + 30].try_into().unwrap()) as usize;
            central += 46 + name;
        }
        fixture.replace_archive(&bytes);
        let result = fixture.run(&mut fixture.command()).await;
        assert!(!result.status.success());
        assert!(
            String::from_utf8_lossy(&result.stdout).contains("Verifying Kuru release archive."),
            "validation not reached: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(
            stderr_message(&result).contains(diagnostic),
            "unexpected rejection: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        fixture.unchanged();
        assert_eq!(
            regular_file_info(&fs::File::open(fixture.install.join("kuru.exe")).unwrap())
                .unwrap()
                .identity,
            identity
        );
    }
}

#[tokio::test]
async fn aliases_hardlinks_private_acl_and_busy_install_lease_fail_closed() {
    let fixture = Fixture::new();
    for path in [
        fixture.install.join("NUL"),
        fixture.install.join("trailing."),
        fixture.install.join("alternate:stream"),
    ] {
        let result = fixture
            .run(fixture.command().arg("-InstallDir").arg(path))
            .await;
        assert!(!result.status.success());
        fixture.unchanged();
        assert!(!fixture.install.join("trailing").exists());
    }
    let other = fixture.root.path().join("other-link.exe");
    fs::hard_link(fixture.install.join("kuru.exe"), &other).unwrap();
    let result = fixture.run(&mut fixture.command()).await;
    assert!(!result.status.success());
    assert!(
        stderr_message(&result).contains("hard-linked files are forbidden"),
        "hardlink validation was not reached: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(fs::read(&other).unwrap(), fixture.original);
    fs::remove_file(other).unwrap();
    fixture.unchanged();
    let state = Directory::open(
        &fixture.install.join(".kuru-update"),
        Privacy::OwnerOnly,
        NameRetention::Pinned,
    )
    .unwrap();
    let lease = state.lock_file(OsStr::new("install.lock")).unwrap();
    lease.try_lock().unwrap();
    let result = fixture.run(&mut fixture.command()).await;
    assert!(!result.status.success());
    assert!(stderr_message(&result).contains("owns the installation"));
    fixture.unchanged();
    drop(lease);
    let result = fixture.run(&mut fixture.command()).await;
    success(&result);
    fixture.installed(&fixture.install);

    let weak = Fixture::new();
    let result = weak.run(&mut weak.script(r#"
$ErrorActionPreference = 'Stop'
$path = [IO.Path]::Combine($env:KURU_INSTALL_DIR, '.kuru-update')
[IO.Directory]::CreateDirectory($path) | Out-Null
$acl = [Security.AccessControl.DirectorySecurity]::new()
$acl.SetAccessRuleProtection($true, $false)
$acl.SetOwner([Security.Principal.WindowsIdentity]::GetCurrent().User)
$rule = [Security.AccessControl.FileSystemAccessRule]::new([Security.Principal.SecurityIdentifier]::new('S-1-1-0'), 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow')
$acl.AddAccessRule($rule)
[IO.Directory]::SetAccessControl($path, $acl)
& $env:KURU_BOOTSTRAP_SCRIPT -Version '0.2.0'
"#)).await;
    assert!(!result.status.success());
    assert!(
        stderr_message(&result).contains("private ACL grants another principal"),
        "weak ACL rejection: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    weak.unchanged();
    assert!(!weak.install.join(".kuru-update/receipt.json").exists());
}

#[tokio::test]
async fn actual_post_move_fault_retains_stage_and_reports_new_namespace_without_rollback_guess() {
    let fixture = Fixture::new();
    let result = fixture.run(&mut fixture.script(r#"
$ErrorActionPreference = 'Stop'
. $env:KURU_BOOTSTRAP_SCRIPT -Recover
$fixtureParent = [Kuru.Bootstrap.Native+DirectoryLease]::new($env:KURU_INSTALL_DIR, $false, $false)
$fixtureStage = [Kuru.Bootstrap.Native+DirectoryLease]::new($fixtureParent.Child('.kuru-install-fault'), $true, $true)
try {
    $payload = [IO.File]::ReadAllBytes($env:KURU_TEST_CANDIDATE)
    $failed = $false
    try { [Kuru.Bootstrap.Native]::Publish($fixtureStage, $fixtureParent, $payload, [Action]{ throw 'fixture failure after actual write-through move' }) }
    catch { $failed = $true }
    if (-not $failed -or -not $fixtureStage.PublicationUncertain) { throw 'Native post-move fault did not retain uncertainty.' }
    $preserved = $false
    try { [Kuru.Bootstrap.Native]::RemoveStage($fixtureStage) }
    catch { $preserved = $true }
    if (-not $preserved -or -not [IO.Directory]::Exists($fixtureStage.Path)) { throw 'Uncertain stage was removed.' }
    if ([Kuru.Bootstrap.Native]::Hash([IO.File]::ReadAllBytes($fixtureParent.Child('kuru.exe'))) -cne [Kuru.Bootstrap.Native]::Hash($payload)) { throw 'Post-move namespace lost installed bytes.' }
} finally { $fixtureStage.Dispose(); $fixtureParent.Dispose() }
"#)).await;
    success(&result);
    assert_eq!(
        fs::read(fixture.install.join("kuru.exe")).unwrap(),
        fixture.replacement
    );
    assert!(fixture.install.join(".kuru-install-fault").is_dir());
    assert!(!fixture.root.path().join("executed").exists());
}

#[tokio::test]
async fn omitted_version_custom_base_has_a_deliberate_error_before_download_or_publication() {
    let fixture = Fixture::new();
    let mut command = Command::new(fixture.powershell());
    command
        .args(fixture.arguments())
        .current_dir(fixture.root.path())
        .env_clear()
        .envs(fixture.environment());
    let result = fixture.run(&mut command).await;
    assert!(!result.status.success());
    assert!(stderr_message(&result).contains("custom release base requires -Version"));
    fixture.unchanged();
}

#[tokio::test]
async fn omitted_version_freezes_simulated_release_roots_before_fetching_real_native_zip() {
    let fixture = Fixture::new();
    let upstream = fixture.root.path().join("simulated upstream λ");
    let latest = upstream.join("latest/download");
    let frozen = upstream.join("download/v0.2.0");
    fs::create_dir_all(&latest).unwrap();
    fs::create_dir_all(&frozen).unwrap();
    let name = archive_name(VERSION, *TARGET).unwrap();
    fs::copy(fixture.release.join(&name), frozen.join(&name)).unwrap();
    let support_name = shell_support::archive_name(VERSION, *TARGET).unwrap();
    fs::copy(
        fixture.release.join(&support_name),
        frozen.join(&support_name),
    )
    .unwrap();
    let manifest = fs::read(fixture.release.join("SHA256SUMS")).unwrap();
    fs::write(latest.join("SHA256SUMS"), &manifest).unwrap();
    // The actual bootstrap is unchanged except its fixed release-origin literal.
    // This proves native selection/local transport, not public HTTPS availability.
    let source =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("support/install.ps1"))
            .unwrap();
    let origin = "https://github.com/replygirl/kuru/releases";
    assert_eq!(source.matches(origin).count(), 3);
    let literal = upstream
        .to_str()
        .unwrap()
        .replace('`', "``")
        .replace('$', "`$")
        .replace('"', "`\"");
    let mut bytes = vec![0xef, 0xbb, 0xbf]; // Stock PS5.1 needs BOM for Unicode source.
    bytes.extend_from_slice(source.replace(origin, &literal).as_bytes());
    let bootstrap = fixture.root.path().join("simulated bootstrap.ps1");
    fs::write(&bootstrap, bytes).unwrap();
    let command = || {
        let mut command = Command::new(fixture.powershell());
        command
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-File"])
            .arg(&bootstrap)
            .current_dir(fixture.root.path())
            .env_clear()
            .envs(fixture.environment())
            .env_remove("KURU_RELEASE_BASE");
        command
    };
    let mut ambiguous = manifest.clone();
    ambiguous
        .extend_from_slice(format!("{}  kuru-0.3.0-{}.zip\n", "0".repeat(64), *TARGET).as_bytes());
    for invalid in [ambiguous, vec![b'x'; 65537]] {
        fs::write(latest.join("SHA256SUMS"), invalid).unwrap();
        assert!(!fixture.run(&mut command()).await.status.success());
        fixture.unchanged();
    }
    fs::write(latest.join("SHA256SUMS"), manifest).unwrap();
    assert!(
        !latest.join(&name).exists(),
        "fixture must fail if version freeze is omitted"
    );
    success(&fixture.run(&mut command()).await);
    fixture.installed(&fixture.install);
}

#[tokio::test]
async fn native_junction_and_incompatible_destination_sharing_preserve_old_bytes() {
    let fixture = Fixture::new();
    let junction = fixture.root.path().join("reparse install");
    // Literal, deliberately authored source; native owned system cmd is only a
    // fixture for creating the real junction, not an installer dependency.
    let mut command = Command::new(system_directory().unwrap().join("cmd.exe"));
    command.args(["/d", "/c"]).arg(format!(
        "mklink /J \"{}\" \"{}\"",
        junction.display(),
        fixture.install.display()
    ));
    success(&output(&mut command, Duration::from_secs(20)).await.unwrap());
    let result = fixture
        .run(fixture.command().arg("-InstallDir").arg(&junction))
        .await;
    assert!(!result.status.success());
    fixture.unchanged();
    fs::remove_dir(&junction).unwrap();
    use std::os::windows::fs::OpenOptionsExt;
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(fixture.install.join("kuru.exe"))
        .unwrap();
    let result = fixture.run(&mut fixture.command()).await;
    assert!(!result.status.success());
    fixture.unchanged();
    drop(held);
    success(&fixture.run(&mut fixture.command()).await);
    fixture.installed(&fixture.install);
}

#[tokio::test]
async fn killing_actual_bootstrap_during_large_verified_archive_work_keeps_old_bytes() {
    let mut fixture = Fixture::new();
    // Real PE with a bounded overlay makes the decode/CRC stage substantial;
    // the acknowledgment precedes decode, not a sleep or guessed startup time.
    fixture.replacement.resize(96 * 1024 * 1024, 0x5a);
    fs::write(&fixture.candidate, &fixture.replacement).unwrap();
    let path = package(&fixture.candidate, *TARGET, VERSION, &fixture.release).unwrap();
    fs::copy(
        path.with_extension("zip.sha256"),
        fixture.release.join("SHA256SUMS"),
    )
    .unwrap();
    let mut child = fixture.native().spawn().await.unwrap();
    let mut stdout = child.take_stdout().unwrap();
    let mut stderr = child.take_stderr().unwrap();
    assert_eq!(
        line(&mut stdout).await.trim(),
        "Verifying Kuru release archive."
    );
    assert!(child.try_wait().unwrap().is_none());
    assert!(fixture.install.join(".kuru-update/install.lock").is_file());
    terminate(&mut child, &mut stdout, &mut stderr).await;
    fixture.unchanged();
    let state = Directory::open(
        &fixture.install.join(".kuru-update"),
        Privacy::OwnerOnly,
        NameRetention::Pinned,
    )
    .unwrap();
    let lease = state.lock_file(OsStr::new("install.lock")).unwrap();
    lease
        .try_lock()
        .expect("cancelled bootstrap leaked its native lock");
}

#[tokio::test]
async fn normal_bootstrap_recovers_genuine_missing_path_crash_gap_before_installing() {
    let fixture = Fixture::new();
    fixture.crash_gap().await;
    let receipt: serde_json::Value = serde_json::from_slice(
        &fs::read(fixture.install.join(".kuru-update/receipt.json")).unwrap(),
    )
    .unwrap();
    // The updater names the helper by its own compiled target. On x64 this is
    // the x64-named helper receipt; recovery never compares it with the
    // native installation target.
    assert_eq!(
        Path::new(receipt["helper"].as_str().unwrap()).file_name(),
        Some(OsStr::new(&format!(
            "{}-{}.exe",
            *TARGET,
            digest(&fixture.original)
        )))
    );
    let result = fixture.run(&mut fixture.command()).await;
    success(&result);
    fixture.installed(&fixture.install);
    assert!(!fixture.install.join(".kuru-update/receipt.json").exists());
    assert_eq!(
        fs::read(receipt["helper"].as_str().unwrap()).unwrap(),
        fixture.original
    );
}

#[tokio::test]
async fn observer_stdin_eof_after_old_move_recovers_exact_original_identity() -> anyhow::Result<()>
{
    use anyhow::{Context, ensure};
    use tokio::io::AsyncBufReadExt;

    let fixture = Fixture::new();
    let installed = fixture.install.join("kuru.exe");
    let original = regular_file_info(&fs::File::open(&installed)?)?.identity;
    let adjacent = fixture.install.join("adjacent");
    fs::write(&adjacent, b"unrelated bytes")?;
    let mut spec = NativeSpawnSpec::new(installed.clone(), fixture.install.clone());
    spec.args = vec![
        "crash-gap".into(),
        fixture.candidate.clone().into(),
        fixture.root.path().join("trusted helper cache").into(),
    ];
    spec.environment = fixture.environment();
    spec.stdin = Stdio::Pipe;
    spec.stdout = Stdio::Pipe;
    spec.stderr = Stdio::Pipe;
    let mut child = spec.spawn().await?;
    let mut input = child.take_stdin().context("crash-gap lifetime input")?;
    let mut stdout = child.take_stdout().context("crash-gap acknowledgment")?;
    let mut stderr = child.take_stderr().context("crash-gap diagnostics")?;
    let bound = launch_budget::until_job_deadline();
    let operation = async {
        let mut marker = Vec::new();
        tokio::time::timeout(bound, async {
            let mut reader = tokio::io::BufReader::new((&mut stdout).take(65537));
            reader.read_until(b'\n', &mut marker).await
        })
        .await
        .context("crash-gap acknowledgment deadline")??;
        ensure!(marker.len() <= 65536 && marker.last() == Some(&b'\n'));
        let marker: serde_json::Value = serde_json::from_slice(&marker)?;
        ensure!(
            marker["ready"] == "old_moved",
            "unexpected marker: {marker}"
        );
        // Only the completed durable move acknowledgment authorizes EOF.
        input.close(Duration::from_secs(10)).await?;
        let mut diagnostic = Vec::new();
        let mut bounded_stderr = (&mut stderr).take(65537);
        let (status, read) = tokio::join!(
            child.wait(bound),
            tokio::time::timeout(bound, bounded_stderr.read_to_end(&mut diagnostic))
        );
        let status = status?;
        read.context("crash-gap diagnostic EOF deadline")??;
        ensure!(
            diagnostic.len() <= 65536,
            "crash-gap diagnostic exceeds bounds"
        );
        Ok::<_, anyhow::Error>((marker, status, diagnostic))
    }
    .await;
    let termination = operation.is_err().then(|| child.terminate());
    let input_close = input.close(Duration::from_secs(10)).await;
    let stdout_close = stdout.close(Duration::from_secs(10)).await;
    let stderr_close = stderr.close(Duration::from_secs(10)).await;
    let reaped = child.wait(Duration::from_secs(10)).await;
    let cleanup = format!(
        "terminate={termination:?}; input={input_close:?}; stdout={stdout_close:?}; stderr={stderr_close:?}; reap={reaped:?}"
    );
    let (marker, status, diagnostic) = operation.context(cleanup)?;
    input_close?;
    stdout_close?;
    stderr_close?;
    reaped?;

    let receipt_path = fixture.install.join(".kuru-update/receipt.json");
    let reported_path = marker["receipt"]
        .as_str()
        .context("reported receipt path")?;
    let reported_receipt = fs::File::open(reported_path)?;
    let expected_receipt = fs::File::open(&receipt_path)?;
    let same_receipt = regular_file_info(&reported_receipt)?.identity
        == regular_file_info(&expected_receipt)?.identity;
    drop((reported_receipt, expected_receipt));
    let receipt: serde_json::Value = serde_json::from_slice(&fs::read(&receipt_path)?)?;
    let name = receipt["displaced"].as_str().context("displaced name")?;
    kuru_platform::fs::validate_component(OsStr::new(name))?;
    let displaced = fixture.install.join(name);
    let displaced_identity = regular_file_info(&fs::File::open(&displaced)?)?.identity;
    let displaced_bytes = fs::read(&displaced)?;
    let missing = !installed.try_exists()?;
    let recovery = fixture.run(fixture.command().arg("-Recover")).await;

    ensure!(!status.success(), "EOF interruption unexpectedly succeeded");
    ensure!(String::from_utf8_lossy(&diagnostic).contains("failed to fill whole buffer"));
    ensure!(same_receipt, "acknowledgment named a different receipt");
    ensure!(missing && displaced_identity == original && displaced_bytes == fixture.original);
    success(&recovery);
    fixture.unchanged();
    ensure!(regular_file_info(&fs::File::open(&installed)?)?.identity == original);
    ensure!(!displaced.try_exists()?);
    let recovered: serde_json::Value = serde_json::from_slice(&fs::read(&receipt_path)?)?;
    ensure!(recovered["phase"] == "rolled_back");
    ensure!(recovered["operation"] == receipt["operation"]);
    ensure!(recovered["original"] == receipt["original"]);
    for field in ["candidate", "backup"] {
        let name = recovered[field].as_str().context("recovered image name")?;
        kuru_platform::fs::validate_component(OsStr::new(name))?;
        ensure!(
            !fixture
                .install
                .join(".kuru-update")
                .join(name)
                .try_exists()?
        );
    }
    ensure!(fs::read(adjacent)? == b"unrelated bytes");
    Ok(())
}

#[tokio::test]
async fn recover_only_restores_exact_old_identity_and_rejects_corrupted_trusted_helper() {
    let fixture = Fixture::new();
    let identity = regular_file_info(&fs::File::open(fixture.install.join("kuru.exe")).unwrap())
        .unwrap()
        .identity;
    fixture.crash_gap().await;
    let receipt: serde_json::Value = serde_json::from_slice(
        &fs::read(fixture.install.join(".kuru-update/receipt.json")).unwrap(),
    )
    .unwrap();
    let helper = Path::new(receipt["helper"].as_str().unwrap());
    let mut corrupt = fixture.original.clone();
    corrupt[0] ^= 1;
    // The real cached helper is deliberately sealed. Preserve that object,
    // present a private corrupt substitute, then restore its original identity;
    // do not grant write access to an immutable trusted executable for a test.
    let cache = Directory::open(
        helper.parent().unwrap(),
        Privacy::OwnerOnly,
        NameRetention::Pinned,
    )
    .unwrap();
    let helper_name = helper.file_name().unwrap();
    let retained_name = OsStr::new("retained-original.exe");
    // Keep the private cache directory pinned, but allow its checked source
    // file to move. Reading through the pinned view denies delete sharing and
    // would make the fixture itself prevent the deliberate substitution.
    let movable = Directory::open(
        helper.parent().unwrap(),
        Privacy::OwnerOnly,
        NameRetention::Movable,
    )
    .unwrap();
    assert_eq!(cache.identity(), movable.identity());
    let original = movable.read(helper_name).unwrap();
    cache
        .rename_file(
            &movable,
            helper_name,
            &original,
            retained_name,
            Publication::New,
        )
        .unwrap();
    let mut substituted = cache.create_new(helper_name).unwrap();
    substituted.write_all(&corrupt).unwrap();
    seal_private(&substituted, true).unwrap();
    substituted.sync_all().unwrap();
    drop(substituted);
    let result = fixture.run(fixture.command().arg("-Recover")).await;
    assert!(!result.status.success());
    assert!(
        stderr_message(&result).contains("Trusted helper identity or checksum changed"),
        "corrupt helper rejection: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!fixture.install.join("kuru.exe").exists());
    cache
        .remove_file(helper_name, movable.read(helper_name).unwrap())
        .unwrap();
    cache
        .rename_file(
            &movable,
            retained_name,
            &original,
            helper_name,
            Publication::New,
        )
        .unwrap();
    drop(original);
    drop(movable);
    drop(cache);
    let result = fixture.run(fixture.command().arg("-Recover")).await;
    success(&result);
    fixture.unchanged();
    assert_eq!(
        regular_file_info(&fs::File::open(fixture.install.join("kuru.exe")).unwrap())
            .unwrap()
            .identity,
        identity
    );
}

#[tokio::test]
async fn bootstrap_selects_the_native_machine_target_whatever_the_architecture_environment_reports()
{
    // AMD64 is true on x64 and false on Arm64; ARM64 is the reverse, so every
    // runner is asked to report at least one contradicting value. x86, an empty
    // value and an unknown name cover reports that name no catalog target at
    // all. It does not launch the bootstrap from an emulated process (design
    // D2, task 3.7).
    //
    // The child records the environment it actually received before it runs
    // the bootstrap, so the injection record never depends on the bootstrap,
    // which deliberately never reads these variables. Both variables are in
    // the child's creation block, but a native x64 runner delivered
    // PROCESSOR_ARCHITECTURE exactly and PROCESSOR_ARCHITEW6432 empty, so the
    // WOW64 variable may be absent but never different. The empty case
    // displaces the fixture's native value, so observing it empty still proves
    // the injection.
    //
    // On x64 every injected PROCESSOR_ARCHITECTURE must arrive exactly, and
    // that run is the proof that the environment is ignored. Windows on Arm
    // did not deliver it: in job 108616153749 (run 36317654214, windows-11-arm)
    // the native ARM64 (0xAA64) stock PowerShell child created with
    // PROCESSOR_ARCHITECTURE=AMD64 and PROCESSOR_ARCHITEW6432=AMD64 observed
    // PROCESSOR_ARCHITECTURE=ARM64 and PROCESSOR_ARCHITEW6432 empty, while the
    // bootstrap still selected aarch64-pc-windows-msvc and installed. That is
    // the only Arm case observed so far; the others are unobserved until a run
    // reaches them. On an aarch64 target a case therefore passes when the child
    // observed either the injected value or the native ARM64 name, and each
    // case's outcome (injected, os-normalised or indistinguishable) is carried
    // in every later assertion message. On Arm the normalised cases run the
    // same bootstrap with the native value present, so they prove native
    // selection and the PE machine but not that a contradicting value is
    // ignored; the identical script's x64 run carries that property.
    let fixture = Fixture::new();
    let (machine, _) = pe_machine(*TARGET);
    let native = kuru_delivery::mise_isolation::native_processor_architecture();
    let mut received = Vec::new();
    for (reported, label) in [
        ("AMD64", "AMD64"),
        ("ARM64", "ARM64"),
        ("x86", "x86"),
        ("", "empty"),
        ("KURU-NOT-A-MACHINE", "unknown"),
    ] {
        let destination = fixture.root.path().join(format!("reported {label}"));
        // `Write-Output` (Utility) runs before the bootstrap's own imports.
        let mut command = fixture.script(&stock_powershell::with_module_prelude(&format!(
            r#"
$ErrorActionPreference = 'Stop'
$reported = [Environment]::GetEnvironmentVariable('PROCESSOR_ARCHITECTURE', 'Process')
$wow64 = [Environment]::GetEnvironmentVariable('PROCESSOR_ARCHITEW6432', 'Process')
Write-Output "observed=[$reported]"
Write-Output "observed-wow64=[$wow64]"
& $env:KURU_BOOTSTRAP_SCRIPT -Version '{VERSION}' -InstallDir $env:KURU_TEST_DESTINATION
Write-Output "native=$([Kuru.Bootstrap.Native]::MachineTarget([Kuru.Bootstrap.Native]::NativeMachine()))"
"#
        )));
        command
            .env("PROCESSOR_ARCHITECTURE", reported)
            .env("PROCESSOR_ARCHITEW6432", reported)
            .env("KURU_TEST_DESTINATION", &destination);
        let result = fixture.run(&mut command).await;
        success(&result);
        let stdout = String::from_utf8_lossy(&result.stdout);
        let lines = stdout.lines().map(str::trim).collect::<Vec<_>>();
        let injected = lines.contains(&format!("observed=[{reported}]").as_str());
        let normalised = cfg!(target_arch = "aarch64")
            && lines.contains(&format!("observed=[{native}]").as_str());
        received.push((
            label,
            match (injected, normalised) {
                (true, true) => "indistinguishable",
                (true, false) => "injected",
                (false, true) => "os-normalised",
                (false, false) => "neither",
            },
        ));
        assert!(
            injected || normalised,
            "the child shell observed neither PROCESSOR_ARCHITECTURE={reported:?} nor, on Arm, \
             the native {native:?}: {stdout}\nreceived so far: {received:?}"
        );
        assert!(
            lines.contains(&"observed-wow64=[]")
                || lines.contains(&format!("observed-wow64=[{reported}]").as_str()),
            "the child shell received a PROCESSOR_ARCHITEW6432 other than {reported:?}: \
             {stdout}\nreceived so far: {received:?}"
        );
        assert!(
            lines.contains(&format!("native={}", *TARGET).as_str()),
            "bootstrap did not detect the native machine with {reported:?} reported: \
             {stdout}\nreceived so far: {received:?}"
        );
        fixture.installed(&destination);
        let installed = fs::read(destination.join("kuru.exe")).unwrap();
        let field = machine_field(&installed);
        assert_eq!(
            u16::from_le_bytes(installed[field..field + 2].try_into().unwrap()),
            machine,
            "installed executable is not the native {} machine with {reported:?} reported; \
             received so far: {received:?}",
            *TARGET
        );
    }
    println!(
        "PROCESSOR_ARCHITECTURE received per case on {}: {received:?}",
        *TARGET
    );
    fixture.unchanged();
}

#[tokio::test]
async fn explicit_target_that_differs_from_the_native_machine_fails_before_any_download() {
    let fixture = Fixture::new();
    for requested in [other_target(), "x86_64-pc-windows-gnu"] {
        let result = fixture
            .run(
                fixture
                    .command()
                    .arg("-Target")
                    .arg(requested)
                    .env("PROCESSOR_ARCHITECTURE", "AMD64")
                    .env("KURU_RELEASE_BASE", "https://must-not-contact.invalid"),
            )
            .await;
        assert!(!result.status.success());
        let message = stderr_message(&result);
        for expected in [
            format!("Requested target {requested}"),
            format!("does not match the native machine target {}", *TARGET),
        ] {
            assert!(
                message.contains(&expected),
                "{expected:?} missing: {message}"
            );
        }
        assert!(
            !String::from_utf8_lossy(&result.stdout).contains("Verifying Kuru release archive."),
            "a mismatched target reached archive work"
        );
        fixture.unchanged();
        assert!(
            !fixture.install.join(".kuru-update").exists(),
            "a mismatched target opened installation state"
        );
    }
}

#[tokio::test]
async fn release_executable_for_another_machine_fails_before_publication() {
    let fixture = Fixture::new();
    let (native, name) = pe_machine(*TARGET);
    let mut foreign = fixture.replacement.clone();
    let field = machine_field(&foreign);
    assert_eq!(
        u16::from_le_bytes(foreign[field..field + 2].try_into().unwrap()),
        native,
        "the native fixture executable must carry the native PE machine"
    );
    foreign[field..field + 2].copy_from_slice(&pe_machine(other_target()).0.to_le_bytes());
    let members = [
        WriteMember {
            name: "kuru.exe",
            kind: MemberKind::File,
            bytes: &foreign,
            executable: true,
        },
        WriteMember {
            name: "LICENSE",
            kind: MemberKind::File,
            bytes: b"license text",
            executable: false,
        },
        WriteMember {
            name: "README.md",
            kind: MemberKind::File,
            bytes: b"release without a support marker",
            executable: false,
        },
    ];
    fixture.replace_archive(
        &zip::write(
            &members,
            Limits {
                max_compressed_bytes: MAX_ARCHIVE_BYTES as u64,
                max_expanded_bytes: MAX_ARCHIVE_BYTES as u64,
                allow_ntfs_timestamps: false,
            },
        )
        .unwrap(),
    );
    let result = fixture.run(&mut fixture.command()).await;
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stdout).contains("Verifying Kuru release archive."),
        "PE validation was never reached: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let message = stderr_message(&result);
    for expected in [
        format!("must be an {name} PE32+ executable"),
        format!("for {}", *TARGET),
    ] {
        assert!(
            message.contains(&expected),
            "{expected:?} missing: {message}"
        );
    }
    fixture.unchanged();
}

#[tokio::test]
async fn recovery_rejects_a_helper_whose_name_and_machine_disagree() {
    let fixture = Fixture::new();
    fixture.crash_gap().await;
    let original_receipt = fs::read(fixture.install.join(".kuru-update/receipt.json")).unwrap();
    let mut receipt: serde_json::Value = serde_json::from_slice(&original_receipt).unwrap();
    let helper = PathBuf::from(receipt["helper"].as_str().unwrap());
    let cache = helper.parent().unwrap();
    let sha256 = receipt["helper_image"]["sha256"]
        .as_str()
        .unwrap()
        .to_owned();
    let helper_name = OsString::from(format!("{}-{sha256}.exe", *TARGET));
    assert_eq!(helper.file_name(), Some(helper_name.as_os_str()));
    // The other Windows catalog target is an accepted helper name on its own;
    // it is rejected only because the image's PE machine disagrees.
    let mislabeled = OsString::from(format!("{}-{sha256}.exe", other_target()));
    rename_helper(cache, &helper_name, &mislabeled);
    receipt["helper"] = helper.with_file_name(&mislabeled).to_str().unwrap().into();
    replace_receipt(&fixture.install, &serde_json::to_vec(&receipt).unwrap());
    let result = fixture.run(fixture.command().arg("-Recover")).await;
    assert!(!result.status.success());
    assert!(
        stderr_message(&result).contains("Recorded helper name and executable machine disagree"),
        "mislabeled helper rejection: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!fixture.install.join("kuru.exe").exists());
    assert!(!fixture.root.path().join("executed").exists());

    rename_helper(cache, &mislabeled, &helper_name);
    replace_receipt(&fixture.install, &original_receipt);
    let result = fixture.run(fixture.command().arg("-Recover")).await;
    success(&result);
    fixture.unchanged();
}
