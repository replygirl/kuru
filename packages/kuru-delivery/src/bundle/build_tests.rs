use super::*;
use kuru_archive::zip::{Archive, MemberSpec};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TARGET: &str = "aarch64-pc-windows-msvc";
const LICENSES: &[u8] = b"fixture Godeps LICENSES for every Dolt target";
const ICU_LICENSE: &[u8] = b"fixture Unicode license for ICU";
const LLVM_LICENSE: &[u8] = b"fixture Apache-2.0 WITH LLVM-exception";
const MINGW_LICENSE: &[u8] = b"fixture mingw-w64 runtime license";
const LINUX: Host = Host {
    os: "linux",
    arch: "x86_64",
};
const MACOS: Host = Host {
    os: "macos",
    arch: "aarch64",
};

/// A minimal PE32+ image with one section holding an import table.
pub(super) fn fake_pe(machine: u16, imports: &[&str], delay: &[&str]) -> Vec<u8> {
    let mut image = vec![0_u8; 0x200];
    image[..2].copy_from_slice(b"MZ");
    image[0x3c..0x40].copy_from_slice(&0x40_u32.to_le_bytes());
    image[0x40..0x44].copy_from_slice(b"PE\0\0");
    image[0x44..0x46].copy_from_slice(&machine.to_le_bytes());
    image[0x46..0x48].copy_from_slice(&1_u16.to_le_bytes());
    image[0x54..0x56].copy_from_slice(&240_u16.to_le_bytes());
    let optional = 0x58;
    image[optional..optional + 2].copy_from_slice(&0x20b_u16.to_le_bytes());
    image[optional + 108..optional + 112].copy_from_slice(&16_u32.to_le_bytes());
    // Section data at file offset 0x200 is mapped at RVA 0x1000.
    let mut data = vec![0_u8; 0x400];
    let import_table = 0;
    let delay_table = 0x100;
    let mut names = 0x200;
    for (index, library) in imports.iter().enumerate() {
        let descriptor = import_table + index * 20;
        data[descriptor + 12..descriptor + 16]
            .copy_from_slice(&(0x1000 + names as u32).to_le_bytes());
        data[descriptor..descriptor + 4].copy_from_slice(&1_u32.to_le_bytes());
        data[names..names + library.len()].copy_from_slice(library.as_bytes());
        names += library.len() + 1;
    }
    for (index, library) in delay.iter().enumerate() {
        let descriptor = delay_table + index * 32;
        data[descriptor..descriptor + 4].copy_from_slice(&1_u32.to_le_bytes());
        data[descriptor + 4..descriptor + 8]
            .copy_from_slice(&(0x1000 + names as u32).to_le_bytes());
        data[names..names + library.len()].copy_from_slice(library.as_bytes());
        names += library.len() + 1;
    }
    if !imports.is_empty() {
        image[optional + 120..optional + 124].copy_from_slice(&0x1000_u32.to_le_bytes());
    }
    if !delay.is_empty() {
        image[optional + 216..optional + 220].copy_from_slice(&0x1100_u32.to_le_bytes());
    }
    let section = optional + 240;
    image[section..section + 5].copy_from_slice(b".data");
    image[section + 8..section + 12].copy_from_slice(&0x400_u32.to_le_bytes());
    image[section + 12..section + 16].copy_from_slice(&0x1000_u32.to_le_bytes());
    image[section + 16..section + 20].copy_from_slice(&0x400_u32.to_le_bytes());
    image[section + 20..section + 24].copy_from_slice(&0x200_u32.to_le_bytes());
    image.extend(data);
    image
}

fn good_pe() -> Vec<u8> {
    fake_pe(
        IMAGE_FILE_MACHINE_ARM64,
        &[
            "KERNEL32.dll",
            "ADVAPI32.dll",
            "api-ms-win-crt-runtime-l1-1-0.dll",
        ],
        &["ws2_32.dll"],
    )
}

fn tarball(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::default(),
    ));
    for (name, bytes) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        builder.append_data(&mut header, name, *bytes).unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap()
}

fn icu_tarball() -> Vec<u8> {
    tarball(&[
        ("icu/LICENSE", ICU_LICENSE),
        ("icu/source/runConfigureICU", b"#!/bin/sh\n"),
        ("icu/source/configure", b"#!/bin/sh\n"),
    ])
}

fn digest(bytes: &[u8]) -> String {
    crate::archive::digest(bytes)
}

/// The committed manifest, re-pinned to fixture license, ICU and notice bytes.
fn manifest(icu: &[u8]) -> Value {
    let mut manifest: Value = serde_json::from_str(include_str!(
        "../../../kuru-memory/support/dolt-assets.json"
    ))
    .unwrap();
    for asset in manifest["assets"].as_array_mut().unwrap() {
        asset["license_bytes"] = json!(LICENSES.len());
        asset["license_sha256"] = json!(digest(LICENSES));
    }
    let built = &mut manifest["assets"][5];
    assert_eq!(built["target"], TARGET);
    built["build"]["sources"]["icu"]["bytes"] = json!(icu.len());
    built["build"]["sources"]["icu"]["sha256"] = json!(digest(icu));
    for (index, bytes) in [ICU_LICENSE, LLVM_LICENSE, MINGW_LICENSE]
        .iter()
        .enumerate()
    {
        built["notices"][index]["bytes"] = json!(bytes.len());
        built["notices"][index]["sha256"] = json!(digest(bytes));
    }
    manifest
}

fn pin(manifest: &mut Value, pins: &ObservedPins) {
    let built = &mut manifest["assets"][5];
    built["compressed_bytes"] = json!(pins.compressed_bytes);
    built["archive_sha256"] = json!(pins.archive_sha256);
    built["expanded_bytes"] = json!(pins.expanded_bytes);
    built["executable_bytes"] = json!(pins.executable_bytes);
    built["executable_sha256"] = json!(pins.executable_sha256);
}

#[derive(Clone, Default)]
struct Behavior {
    go_version: Option<String>,
    clang: Option<String>,
    sum: Option<String>,
    version_go: Option<String>,
    licenses: Option<Vec<u8>>,
    stub_bytes: usize,
    pe: Option<Vec<u8>>,
    fail: Option<&'static str>,
}

/// Each recorded step: label, explicit environment and arguments.
type Recorded = (String, HashMap<String, String>, Vec<String>);

struct FakeRunner {
    behavior: Behavior,
    path: OsString,
    steps: Arc<Mutex<Vec<Recorded>>>,
}

impl Runner for FakeRunner {
    fn run<'a>(&'a mut self, step: &'a Step) -> StepFuture<'a> {
        Box::pin(async move {
            let environment: HashMap<String, String> = step
                .env
                .iter()
                .map(|(name, value)| {
                    (
                        name.to_string_lossy().into_owned(),
                        value.to_string_lossy().into_owned(),
                    )
                })
                .collect();
            let arguments: Vec<String> = step
                .args
                .iter()
                .map(|value| value.to_string_lossy().into_owned())
                .collect();
            self.steps.lock().unwrap().push((
                step.label.to_owned(),
                environment.clone(),
                arguments.clone(),
            ));
            let behavior = &self.behavior;
            let success = |stdout: String| {
                Ok(Output {
                    status: exit_status(0),
                    stdout: stdout.into_bytes(),
                    stderr: Vec::new(),
                })
            };
            if behavior.fail == Some(step.label) {
                return Ok(Output {
                    status: exit_status(1),
                    stdout: b"fixture stdout".to_vec(),
                    stderr: b"fixture failure detail".to_vec(),
                });
            }
            match step.label {
                "go version" => success(
                    behavior
                        .go_version
                        .clone()
                        .unwrap_or_else(|| "go version go1.26.2 linux/amd64\n".into()),
                ),
                "clang version" => success(behavior.clang.clone().unwrap_or_else(|| {
                    "clang version 23.1.2 (https://github.com/llvm/llvm-project abc)\nTarget: aarch64-w64-windows-gnu\n".into()
                })),
                "go mod download" => {
                    assert_eq!(environment["GOFLAGS"], "-mod=mod -modcacherw");
                    let cache = PathBuf::from(&environment["GOMODCACHE"]);
                    let module = arguments.last().unwrap().clone();
                    let (path, version) = module.split_once('@').unwrap();
                    let directory = cache.join(format!("{path}@{version}"));
                    fs::create_dir_all(directory.join("cmd/dolt/doltversion")).unwrap();
                    fs::create_dir_all(directory.join("Godeps")).unwrap();
                    fs::write(
                        directory.join("cmd/dolt/doltversion/version.go"),
                        behavior.version_go.clone().unwrap_or_else(|| {
                            "package doltversion\n\nconst (\n\tVersion = \"2.3.3\"\n)\n".into()
                        }),
                    )
                    .unwrap();
                    fs::write(
                        directory.join("Godeps/LICENSES"),
                        behavior.licenses.clone().unwrap_or_else(|| LICENSES.to_vec()),
                    )
                    .unwrap();
                    success(
                        json!({
                            "Path": path,
                            "Version": version,
                            "Sum": behavior.sum.clone().unwrap_or_else(|| "h1:UvzzAyIZXdmbEia/ltgNGDRM30nGMPc7ReWEzalNYmo=".into()),
                            "Dir": directory,
                        })
                        .to_string(),
                    )
                }
                "build cross ICU" => {
                    fs::create_dir_all(step.dir.join("lib")).unwrap();
                    fs::create_dir_all(step.dir.join("stubdata")).unwrap();
                    fs::write(step.dir.join("lib/libsicuin.a"), b"i18n").unwrap();
                    fs::write(step.dir.join("lib/libsicuuc.a"), b"common").unwrap();
                    fs::write(
                        step.dir.join("stubdata/libsicudt.a"),
                        vec![b'!'; behavior.stub_bytes.max(1)],
                    )
                    .unwrap();
                    success(String::new())
                }
                "go build" => {
                    let output = arguments
                        .iter()
                        .position(|value| value == "-o")
                        .map(|index| PathBuf::from(&arguments[index + 1]))
                        .unwrap();
                    fs::write(output, behavior.pe.clone().unwrap_or_else(good_pe)).unwrap();
                    success(String::new())
                }
                _ => success(String::new()),
            }
        })
    }

    fn path(&self) -> Option<&OsStr> {
        Some(&self.path)
    }
}

#[cfg(unix)]
fn exit_status(code: i32) -> std::process::ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(code << 8)
}

#[cfg(windows)]
fn exit_status(code: u32) -> std::process::ExitStatus {
    use std::os::windows::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(code)
}

struct Fixture {
    root: tempfile::TempDir,
    manifest: PathBuf,
    icu: Vec<u8>,
    toolchain_bin: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let icu = icu_tarball();
        let manifest = root.path().join("dolt-assets.json");
        fs::write(
            &manifest,
            serde_json::to_vec(&self::manifest(&icu)).unwrap(),
        )
        .unwrap();
        let toolchain = root.path().join("llvm-mingw");
        let bin = toolchain.join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("aarch64-w64-mingw32-clang"), b"fixture").unwrap();
        fs::write(toolchain.join("LICENSE.TXT"), LLVM_LICENSE).unwrap();
        let mingw = toolchain.join("aarch64-w64-mingw32/share/mingw32");
        fs::create_dir_all(&mingw).unwrap();
        fs::write(mingw.join("COPYING.MinGW-w64-runtime.txt"), MINGW_LICENSE).unwrap();
        Self {
            root,
            manifest,
            icu,
            toolchain_bin: bin,
        }
    }

    fn edit(&self, change: impl FnOnce(&mut Value)) {
        let mut value: Value = serde_json::from_slice(&fs::read(&self.manifest).unwrap()).unwrap();
        change(&mut value);
        fs::write(&self.manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    }

    fn options(&self, run: &str) -> BuildOptions {
        BuildOptions {
            manifest: self.manifest.clone(),
            target: TARGET.into(),
            work_dir: self.root.path().join(run).join("work"),
            output: self.root.path().join(run).join("out"),
            print_pins: true,
            offline: false,
            host_override: false,
            host: LINUX,
            jobs: 2,
        }
    }

    fn runner(&self, behavior: Behavior) -> FakeRunner {
        FakeRunner {
            behavior,
            path: std::env::join_paths([
                self.root.path().join("missing"),
                self.toolchain_bin.clone(),
            ])
            .unwrap(),
            steps: Arc::default(),
        }
    }

    async fn run(&self, options: &BuildOptions, behavior: Behavior) -> Result<BuildReport> {
        let mut runner = self.runner(behavior);
        self.run_with(options, &mut runner, self.icu.clone()).await
    }

    async fn run_with(
        &self,
        options: &BuildOptions,
        runner: &mut FakeRunner,
        served: Vec<u8>,
    ) -> Result<BuildReport> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/icu4c-sources.tgz",
            listener.local_addr().unwrap()
        );
        let server = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    request.push(socket.read_u8().await.unwrap());
                    assert!(request.len() < 8192);
                }
                let mut response = format!(
                    "HTTP/1.1 200 Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    served.len()
                )
                .into_bytes();
                response.extend_from_slice(&served);
                let _ = socket.write_all(&response).await;
            }
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap();
        let result = build_with(options, runner, &client, Some(&url)).await;
        server.abort();
        let _ = server.await;
        result
    }
}

fn error(result: Result<BuildReport>) -> String {
    format!("{:#}", result.expect_err("the build must fail closed"))
}

#[tokio::test]
async fn unpinned_build_reports_pins_and_writes_the_exact_manifest_layout() {
    let fixture = Fixture::new();
    let options = fixture.options("first");
    let mut runner = fixture.runner(Behavior::default());
    let steps = runner.steps.clone();
    let report = fixture
        .run_with(&options, &mut runner, fixture.icu.clone())
        .await
        .unwrap();
    assert!(!report.verified);
    assert!(report.pins.authoritative);
    assert!(summary(&report).contains("unpinned: pin verification not yet possible"));
    assert_eq!(report.pins.go_version, "go version go1.26.2 linux/amd64");
    assert_eq!(
        report.pins.imports,
        [
            "advapi32.dll",
            "api-ms-win-crt-runtime-l1-1-0.dll",
            "kernel32.dll",
            "ws2_32.dll"
        ]
    );
    let archive = fs::read(&report.archive).unwrap();
    assert_eq!(
        report.archive.file_name().unwrap(),
        "dolt-windows-arm64.zip"
    );
    assert_eq!(report.pins.archive_sha256, digest(&archive));
    assert_eq!(report.pins.compressed_bytes, archive.len() as u64);
    let written: ObservedPins =
        serde_json::from_slice(&fs::read(&report.pins_file).unwrap()).unwrap();
    assert_eq!(written, report.pins);
    let executable = good_pe();
    let files: [(&str, &[u8], u32); 5] = [
        ("dolt-windows-arm64/bin/dolt.exe", &executable, 0o100755),
        ("dolt-windows-arm64/LICENSES", LICENSES, 0o100644),
        ("dolt-windows-arm64/LICENSE-ICU", ICU_LICENSE, 0o100644),
        ("dolt-windows-arm64/LICENSE-LLVM", LLVM_LICENSE, 0o100644),
        (
            "dolt-windows-arm64/LICENSE-MINGW-W64-RUNTIME",
            MINGW_LICENSE,
            0o100644,
        ),
    ];
    let mut expected = vec![
        MemberSpec {
            name: "dolt-windows-arm64/",
            kind: MemberKind::Directory,
            max_bytes: 0,
            exact_bytes: Some(0),
            unix_mode: Some(0o040755),
        },
        MemberSpec {
            name: "dolt-windows-arm64/bin/",
            kind: MemberKind::Directory,
            max_bytes: 0,
            exact_bytes: Some(0),
            unix_mode: Some(0o040755),
        },
    ];
    expected.extend(files.iter().map(|(name, bytes, mode)| MemberSpec {
        name,
        kind: MemberKind::File,
        max_bytes: bytes.len() as u64,
        exact_bytes: Some(bytes.len() as u64),
        unix_mode: Some(*mode),
    }));
    let mut decoded = Archive::open(
        &archive,
        &expected,
        Limits {
            max_compressed_bytes: MAX_ARCHIVE,
            max_expanded_bytes: report.pins.expanded_bytes,
            allow_ntfs_timestamps: false,
        },
    )
    .unwrap();
    for (name, bytes, _) in files {
        let mut copied = Vec::new();
        decoded.copy(name, &mut copied).unwrap();
        assert_eq!(copied, bytes, "{name}");
    }
    assert_eq!(
        report.pins.expanded_bytes,
        files
            .iter()
            .map(|(_, bytes, _)| bytes.len() as u64)
            .sum::<u64>()
    );
    // The recipe runs in order, with only its own environment.
    let steps = steps.lock().unwrap();
    let labels: Vec<_> = steps.iter().map(|(label, _, _)| label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "go version",
            "clang version",
            "go mod download",
            "configure host ICU",
            "build host ICU",
            "configure cross ICU",
            "build cross ICU",
            "go build"
        ]
    );
    let (_, environment, arguments) = steps.last().unwrap();
    for (name, value) in [
        ("CGO_ENABLED", "1"),
        ("GOOS", "windows"),
        ("GOARCH", "arm64"),
        ("GOTOOLCHAIN", "local"),
        ("GOFLAGS", "-mod=readonly -modcacherw"),
        ("GOPROXY", "https://proxy.golang.org"),
        ("GOSUMDB", "sum.golang.org"),
        ("CC", "aarch64-w64-mingw32-clang"),
        ("CXX", "aarch64-w64-mingw32-clang++"),
    ] {
        assert_eq!(environment[name], value, "{name}");
    }
    assert!(environment["CGO_CXXFLAGS"].contains("-include cstdlib"));
    assert!(environment["CGO_CXXFLAGS"].contains("-ffile-prefix-map="));
    assert!(environment["CGO_LDFLAGS"].ends_with(" -static"));
    for arg in [
        "-trimpath",
        "-buildvcs=false",
        "icu_static,timetzdata",
        "-ldflags=-s -w -buildid=",
        "./cmd/dolt",
    ] {
        assert!(arguments.iter().any(|value| value == arg), "{arg}");
    }
    let (_, _, configure) = &steps[5];
    for arg in [
        "--host=aarch64-w64-mingw32",
        "--enable-static",
        "--disable-shared",
        "--with-data-packaging=static",
    ] {
        assert!(configure.iter().any(|value| value == arg), "{arg}");
    }
    // Every step sees an explicit environment, never inherited compiler flags.
    for (label, environment, _) in steps.iter() {
        assert!(!environment.contains_key("CFLAGS"), "{label}");
        assert!(!environment.contains_key("PATH"), "{label}");
    }
}

#[tokio::test]
async fn committed_pins_are_verified_and_any_drift_fails_with_both_digests() {
    let fixture = Fixture::new();
    let first = fixture
        .run(&fixture.options("first"), Behavior::default())
        .await
        .unwrap();
    fixture.edit(|manifest| pin(manifest, &first.pins));
    let mut options = fixture.options("second");
    options.print_pins = false;
    let second = fixture.run(&options, Behavior::default()).await.unwrap();
    assert!(second.verified);
    assert!(summary(&second).contains("verified against the committed pins"));
    assert_eq!(
        fs::read(&first.archive).unwrap(),
        fs::read(&second.archive).unwrap(),
        "fresh work directories produce identical archive bytes"
    );
    let drifted = Behavior {
        pe: Some(fake_pe(
            IMAGE_FILE_MACHINE_ARM64,
            &["kernel32.dll", "advapi32.dll"],
            &[],
        )),
        ..Behavior::default()
    };
    let options = fixture.options("third");
    let message = error(fixture.run(&options, drifted).await);
    assert!(
        message.contains("differs from the committed pins"),
        "{message}"
    );
    assert!(
        message.contains(&format!(
            "archive_sha256: pinned {}",
            first.pins.archive_sha256
        )),
        "{message}"
    );
    assert!(message.contains("executable_sha256: pinned"), "{message}");
    assert!(
        options.output.join("pins.json").is_file(),
        "observed pins stay available for diagnosis"
    );
    // A pinned notice that changes is also drift.
    fixture.edit(|manifest| {
        manifest["assets"][5]["notices"][1]["sha256"] = json!("0".repeat(64));
    });
    let message = error(
        fixture
            .run(&fixture.options("fourth"), Behavior::default())
            .await,
    );
    assert!(
        message.contains("LICENSE-LLVM sha256: pinned 0000"),
        "{message}"
    );
}

#[tokio::test]
async fn host_mode_and_provenance_are_refused_before_any_work() {
    let fixture = Fixture::new();
    let mut options = fixture.options("refused");
    options.host = MACOS;
    let message = error(fixture.run(&options, Behavior::default()).await);
    assert!(
        message.contains("runs only on linux-x64, not macos-arm64"),
        "{message}"
    );
    assert!(message.contains("KURU_BUNDLE_BUILD_HOST_OVERRIDE=1"));
    options.host_override = true;
    options.print_pins = false;
    fixture.edit(|manifest| {
        let pins = ObservedPins {
            target: TARGET.into(),
            authoritative: true,
            go_version: String::new(),
            clang_version: String::new(),
            compressed_bytes: 10,
            archive_sha256: "a".repeat(64),
            expanded_bytes: (LICENSES.len()
                + ICU_LICENSE.len()
                + LLVM_LICENSE.len()
                + MINGW_LICENSE.len()
                + 5) as u64,
            executable_bytes: 5,
            executable_sha256: "b".repeat(64),
            license_bytes: 0,
            license_sha256: String::new(),
            notices: Vec::new(),
            imports: Vec::new(),
        };
        pin(manifest, &pins);
    });
    assert!(
        error(fixture.run(&options, Behavior::default()).await)
            .contains("non-authoritative host cannot verify pins")
    );
    let mut options = fixture.options("offline");
    options.offline = true;
    assert!(
        error(fixture.run(&options, Behavior::default()).await)
            .contains("unset KURU_DOLT_BUNDLE_OFFLINE")
    );
    let mut options = fixture.options("upstream");
    options.target = "x86_64-pc-windows-msvc".into();
    assert!(
        error(fixture.run(&options, Behavior::default()).await)
            .contains("uses the upstream Dolt archive")
    );
    let mut options = fixture.options("relative");
    options.work_dir = "relative/work".into();
    assert!(error(fixture.run(&options, Behavior::default()).await).contains("must be absolute"));
    let mut options = fixture.options("nested");
    options.output = options.work_dir.join("out");
    assert!(error(fixture.run(&options, Behavior::default()).await).contains("must be separate"));
    let mut options = fixture.options("jobs");
    options.jobs = 0;
    assert!(error(fixture.run(&options, Behavior::default()).await).contains("at least one job"));
    for run in ["refused", "offline", "upstream", "nested", "jobs"] {
        assert!(
            !fixture.root.path().join(run).exists(),
            "{run} created a work directory"
        );
    }
}

#[tokio::test]
async fn unpinned_asset_requires_print_pins_and_override_is_non_authoritative() {
    let fixture = Fixture::new();
    let mut options = fixture.options("strict");
    options.print_pins = false;
    let message = error(fixture.run(&options, Behavior::default()).await);
    assert!(message.contains("not yet pinned"), "{message}");
    assert!(message.contains("pass --print-pins"), "{message}");
    assert!(!fixture.root.path().join("strict").exists());
    let mut options = fixture.options("override");
    options.host = MACOS;
    options.host_override = true;
    let behavior = Behavior {
        go_version: Some("go version go1.26.2 darwin/arm64\n".into()),
        ..Behavior::default()
    };
    let report = fixture.run(&options, behavior.clone()).await.unwrap();
    assert!(!report.pins.authoritative);
    assert!(!report.verified);
    assert!(summary(&report).contains("non-authoritative host override"));
    // The authoritative host requires the exact linux/amd64 toolchain.
    let message = error(fixture.run(&fixture.options("exact"), behavior).await);
    assert!(message.contains("the recipe pins go1.26.2"), "{message}");
    // A pre-existing work directory is never reused.
    let message = error(
        fixture
            .run(&fixture.options("override"), Behavior::default())
            .await,
    );
    assert!(
        message.contains("fresh private work directory"),
        "{message}"
    );
}

#[tokio::test]
async fn tampered_or_mismatched_inputs_fail_closed() {
    let cases: Vec<(&str, Behavior, &str)> = vec![
        (
            "go",
            Behavior {
                go_version: Some("go version go1.27.0 linux/amd64".into()),
                ..Behavior::default()
            },
            "the recipe pins go1.26.2",
        ),
        (
            "clang",
            Behavior {
                clang: Some("clang version 23.1.20 (fork)".into()),
                ..Behavior::default()
            },
            "the recipe pins clang 23.1.2",
        ),
        (
            "sum",
            Behavior {
                sum: Some("h1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into()),
                ..Behavior::default()
            },
            "does not match the pinned",
        ),
        (
            "version",
            Behavior {
                version_go: Some("const Version = \"2.3.4\"".into()),
                ..Behavior::default()
            },
            "does not declare version 2.3.3",
        ),
        (
            "licenses",
            Behavior {
                licenses: Some(b"different LICENSES".to_vec()),
                ..Behavior::default()
            },
            "does not match the pinned LICENSES",
        ),
        (
            "stub",
            Behavior {
                stub_bytes: 64 * 1024 + 1,
                ..Behavior::default()
            },
            "not the stub",
        ),
        (
            "machine",
            Behavior {
                pe: Some(fake_pe(0x8664, &["kernel32.dll"], &[])),
                ..Behavior::default()
            },
            "not ARM64",
        ),
        (
            "imports",
            Behavior {
                pe: Some(fake_pe(
                    IMAGE_FILE_MACHINE_ARM64,
                    &["kernel32.dll", "libc++.dll"],
                    &[],
                )),
                ..Behavior::default()
            },
            "unapproved external DLL: libc++.dll",
        ),
        (
            "step",
            Behavior {
                fail: Some("build cross ICU"),
                ..Behavior::default()
            },
            "fixture failure detail",
        ),
    ];
    let fixture = Fixture::new();
    for (name, behavior, expected) in cases {
        let message = error(fixture.run(&fixture.options(name), behavior).await);
        assert!(message.contains(expected), "{name}: {message}");
        assert!(
            !fixture
                .root
                .path()
                .join(name)
                .join("out/dolt-windows-arm64.zip")
                .exists(),
            "{name} published an archive"
        );
    }
    let mut runner = fixture.runner(Behavior::default());
    let mut tampered = fixture.icu.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 1;
    let message = error(
        fixture
            .run_with(&fixture.options("icu"), &mut runner, tampered)
            .await,
    );
    assert!(message.contains("checksum mismatch"), "{message}");
    let labels: Vec<_> = runner
        .steps
        .lock()
        .unwrap()
        .iter()
        .map(|(label, _, _)| label.clone())
        .collect();
    assert!(
        !labels.iter().any(|label| label.contains("ICU")),
        "no ICU build runs after a tampered download: {labels:?}"
    );
    // The cross compiler must come from the toolchain on PATH.
    let mut runner = fixture.runner(Behavior::default());
    runner.path = fixture.root.path().join("missing").into_os_string();
    let message = error(
        fixture
            .run_with(&fixture.options("path"), &mut runner, fixture.icu.clone())
            .await,
    );
    assert!(message.contains("is not on PATH"), "{message}");
}

#[test]
fn pe_inspection_requires_arm64_pe32_plus_and_system_imports() {
    assert_eq!(
        check_pe(&good_pe()).unwrap(),
        [
            "advapi32.dll",
            "api-ms-win-crt-runtime-l1-1-0.dll",
            "kernel32.dll",
            "ws2_32.dll"
        ]
    );
    assert!(check_pe(b"not a PE").is_err());
    let mut unsigned = good_pe();
    unsigned[0x40] = b'X';
    assert!(
        check_pe(&unsigned)
            .unwrap_err()
            .to_string()
            .contains("PE signature")
    );
    let mut pe32 = good_pe();
    pe32[0x58] = 0x0b;
    pe32[0x59] = 0x01;
    assert!(check_pe(&pe32).unwrap_err().to_string().contains("PE32+"));
    assert!(
        check_pe(&fake_pe(IMAGE_FILE_MACHINE_ARM64, &[], &[]))
            .unwrap_err()
            .to_string()
            .contains("no DLL imports")
    );
    let mut outside = good_pe();
    outside[0x58 + 120..0x58 + 124].copy_from_slice(&0x9000_u32.to_le_bytes());
    assert!(
        check_pe(&outside)
            .unwrap_err()
            .to_string()
            .contains("outside every section")
    );
    let mut truncated = good_pe();
    truncated.truncate(0x210);
    assert!(check_pe(&truncated).is_err());
    for library in [
        "api-ms-win-crt-heap-l1-1-0.dll",
        "ext-ms-win-ntuser-window-l1-1-4.dll",
    ] {
        assert!(api_set(library), "{library}");
    }
    for library in [
        "api-ms-win-crt-heap-1-0.dll",
        "api-ms-win--l1-1-0.dll",
        "api-ms-win-crt-heap-lx-1-0.dll",
        "api-ms-win-crt-heap-l1-1-a.dll",
        "api-ms-win-CRT-l1-1-0.dll",
        "msvcp140.dll",
    ] {
        assert!(!api_set(library), "{library}");
    }
}

#[test]
fn source_trees_reject_links_and_unsafe_paths() {
    let root = tempfile::tempdir().unwrap();
    let mut archive = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::default(),
    ));
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_size(0);
    header.set_mode(0o777);
    header.set_cksum();
    archive
        .append_link(&mut header, "icu/link", "/etc/passwd")
        .unwrap();
    let linked = archive.into_inner().unwrap().finish().unwrap();
    assert!(
        extract_sources(&linked, &root.path().join("linked"))
            .unwrap_err()
            .to_string()
            .contains("link or special entry")
    );
    // tar::Builder refuses to write `..`; craft the raw header name instead.
    let mut header = tar::Header::new_old();
    header.as_old_mut().name[..9].copy_from_slice(b"../escape");
    header.set_size(1);
    header.set_mode(0o644);
    header.set_entry_type(tar::EntryType::Regular);
    header.set_cksum();
    let mut raw = header.as_bytes().to_vec();
    raw.extend_from_slice(&[b'x'; 512]);
    raw.extend_from_slice(&[0; 1024]);
    let mut escaped = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    escaped.write_all(&raw).unwrap();
    let escaped = escaped.finish().unwrap();
    assert!(
        extract_sources(&escaped, &root.path().join("escaped"))
            .unwrap_err()
            .to_string()
            .contains("unsafe path")
    );
    assert!(!root.path().join("escape").exists());
    #[cfg(unix)]
    {
        let module = root.path().join("module");
        fs::create_dir_all(module.join("dir")).unwrap();
        fs::write(module.join("dir/file"), b"source").unwrap();
        std::os::unix::fs::symlink("/etc/passwd", module.join("link")).unwrap();
        assert!(
            copy_tree(&module, &root.path().join("copy"))
                .unwrap_err()
                .to_string()
                .contains("link or special file")
        );
    }
    assert!(read_bounded(&root.path().join("absent"), 10).is_err());
    fs::write(root.path().join("large"), [0_u8; 11]).unwrap();
    assert!(read_bounded(&root.path().join("large"), 10).is_err());
    assert!(toolchain_root(None, "clang").is_err());
}

#[test]
fn process_runner_inherits_only_path_and_home() {
    let runner = ProcessRunner::new([
        (OsString::from("PATH"), OsString::from("/fixture/bin")),
        (OsString::from("HOME"), OsString::from("/fixture/home")),
        (OsString::from("CC"), OsString::from("evil-cc")),
        (OsString::from("CGO_LDFLAGS"), OsString::from("-lhostile")),
        (OsString::from("GOFLAGS"), OsString::from("-mod=mod")),
        (OsString::from("GOPRIVATE"), OsString::from("*")),
    ]);
    let names: Vec<_> = runner
        .inherited
        .iter()
        .map(|(name, _)| name.to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["PATH", "HOME"]);
    assert_eq!(runner.path(), Some(OsStr::new("/fixture/bin")));
}

#[cfg(unix)]
#[tokio::test]
async fn process_runner_clears_the_inherited_environment_of_real_children() {
    let root = tempfile::tempdir().unwrap();
    let mut runner = ProcessRunner::new(
        std::env::vars_os()
            .filter(|(name, _)| name == "PATH")
            .chain([
                (OsString::from("CC"), OsString::from("evil-cc")),
                (OsString::from("CFLAGS"), OsString::from("-DHOSTILE")),
            ]),
    );
    let step = Step {
        label: "environment",
        program: "sh".into(),
        args: args(["-c", "env; printf 'fixture stderr' >&2"]),
        dir: root.path().to_owned(),
        env: vec![env("GOTOOLCHAIN", "local")],
        timeout: Duration::from_secs(10),
    };
    let output = runner.run(&step).await.unwrap();
    assert!(output.status.success());
    let printed = String::from_utf8(output.stdout).unwrap();
    assert!(printed.contains("GOTOOLCHAIN=local"), "{printed}");
    assert!(printed.contains("PATH="), "{printed}");
    assert!(
        !printed.contains("evil-cc") && !printed.contains("HOSTILE"),
        "{printed}"
    );
    assert_eq!(output.stderr, b"fixture stderr");
    let failing = Step {
        label: "failing",
        program: "sh".into(),
        args: args(["-c", "echo out; echo err >&2; exit 3"]),
        dir: root.path().to_owned(),
        env: Vec::new(),
        timeout: Duration::from_secs(10),
    };
    let message = checked(&mut runner, &failing)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        message.contains("failing failed") && message.contains("err"),
        "{message}"
    );
    let missing = Step {
        label: "missing tool",
        program: "kuru-definitely-missing-tool".into(),
        args: Vec::new(),
        dir: root.path().to_owned(),
        env: Vec::new(),
        timeout: Duration::from_secs(10),
    };
    assert!(format!("{:#}", runner.run(&missing).await.unwrap_err()).contains("run missing tool"));
}
