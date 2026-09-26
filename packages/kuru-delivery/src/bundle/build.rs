//! Reproducible source builds for Dolt targets that upstream never publishes.
//!
//! The pinned recipe runs only on linux-x64: llvm-mingw's target runtimes embed
//! host-package paths, so another host produces different bytes. Every source
//! input is verified against the manifest before it is used, the child
//! environment is rebuilt from a small allowlist, and the archive is written in
//! exactly the layout that runtime extraction accepts.

use super::{
    Asset, BUILD_HOST, MAX_ARCHIVE, MAX_EXPANDED, MAX_NOTICE, ManifestAsset, NoticeSource,
    Provenance, RETRY_DELAYS, UNPINNED, download, load_manifest, relative_path,
};
use anyhow::{Context, Result, bail, ensure};
use kuru_archive::zip::{Limits, MemberKind, WriteMember};
use kuru_platform::fs::Directory as CheckedDirectory;
use serde::{Deserialize, Serialize};
use std::{
    ffi::{OsStr, OsString},
    fs,
    future::Future,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    pin::Pin,
    process::Output,
    time::Duration,
};

/// Environment variables a build child may inherit. Everything else, including
/// compiler flags, `CC`/`CXX`, `CGO_*`, `GOFLAGS` and private-module settings,
/// is cleared so the maintainer's shell cannot change the bytes.
const INHERITED: [&str; 2] = ["PATH", "HOME"];
const CROSS_HOST: &str = "aarch64-w64-mingw32";
const PREFIX_MAP: &str = "/kuru-build";
const OUTPUT_LIMIT: usize = 64 * 1024 * 1024;
const QUICK: Duration = Duration::from_secs(60);
const FETCH: Duration = Duration::from_secs(30 * 60);
const COMPILE: Duration = Duration::from_secs(90 * 60);
const ICU_DOWNLOAD: Duration = Duration::from_secs(10 * 60);
const MAX_SOURCE_TREE: u64 = 1024 * 1024 * 1024;
const MAX_SOURCE_ENTRIES: usize = 100_000;
const MAX_STUB_DATA: u64 = 64 * 1024;
const IMAGE_FILE_MACHINE_ARM64: u16 = 0xaa64;

/// Windows 10/11 system DLLs, identical to the shipping-image check in
/// `apps/kuru-tui/support/verify-windows-imports.ps1`.
const SYSTEM_LIBRARIES: [&str; 25] = [
    "advapi32.dll",
    "bcrypt.dll",
    "bcryptprimitives.dll",
    "combase.dll",
    "crypt32.dll",
    "dbgcore.dll",
    "dbghelp.dll",
    "dnsapi.dll",
    "iphlpapi.dll",
    "kernel32.dll",
    "msvcrt.dll",
    "ncrypt.dll",
    "ntdll.dll",
    "ole32.dll",
    "oleaut32.dll",
    "rpcrt4.dll",
    "secur32.dll",
    "shell32.dll",
    "shlwapi.dll",
    "ucrtbase.dll",
    "user32.dll",
    "userenv.dll",
    "version.dll",
    "winhttp.dll",
    "ws2_32.dll",
];

/// The platform this helper runs on, compared against the manifest build host.
#[derive(Clone, Copy, Debug)]
pub struct Host {
    pub os: &'static str,
    pub arch: &'static str,
}

impl Host {
    pub const fn current() -> Self {
        Self {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
        }
    }

    fn name(self) -> String {
        let arch = match self.arch {
            "x86_64" => "x64",
            "aarch64" => "arm64",
            other => other,
        };
        format!("{}-{arch}", self.os)
    }
}

#[derive(Debug)]
pub struct BuildOptions {
    pub manifest: PathBuf,
    pub target: String,
    /// Fresh private work directory; it must not already exist.
    pub work_dir: PathBuf,
    /// Private output directory receiving `<stem>.zip` and `pins.json`.
    pub output: PathBuf,
    pub print_pins: bool,
    pub offline: bool,
    /// `KURU_BUNDLE_BUILD_HOST_OVERRIDE=1`: build on another host for local
    /// iteration. Its bytes are never authoritative and never become pins.
    pub host_override: bool,
    pub host: Host,
    pub jobs: usize,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedNotice {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}

/// Pins observed from one build, printed for the manifest's round-two update.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedPins {
    pub target: String,
    pub authoritative: bool,
    pub go_version: String,
    pub clang_version: String,
    pub compressed_bytes: u64,
    pub archive_sha256: String,
    pub expanded_bytes: u64,
    pub executable_bytes: u64,
    pub executable_sha256: String,
    pub license_bytes: u64,
    pub license_sha256: String,
    pub notices: Vec<ObservedNotice>,
    pub imports: Vec<String>,
}

#[derive(Debug)]
pub struct BuildReport {
    pub archive: PathBuf,
    pub pins_file: PathBuf,
    pub pins: ObservedPins,
    /// False when the manifest is not yet pinned and `--print-pins` reported
    /// the observed identity instead of verifying it.
    pub verified: bool,
}

/// One child process invocation. The runner adds only the allowlisted
/// inherited environment, so tests can inspect exactly what a build runs.
#[derive(Debug)]
pub struct Step {
    pub label: &'static str,
    pub program: OsString,
    pub args: Vec<OsString>,
    pub dir: PathBuf,
    pub env: Vec<(OsString, OsString)>,
    pub timeout: Duration,
}

pub type StepFuture<'a> = Pin<Box<dyn Future<Output = Result<Output>> + 'a>>;

/// The injectable process boundary. Production uses [`ProcessRunner`].
pub trait Runner {
    fn run<'a>(&'a mut self, step: &'a Step) -> StepFuture<'a>;
    /// The `PATH` value children see, used to locate the toolchain root.
    fn path(&self) -> Option<&OsStr>;
}

/// Run owned, bounded subprocesses with a cleared environment.
pub struct ProcessRunner {
    inherited: Vec<(OsString, OsString)>,
}

impl ProcessRunner {
    /// Keep only the allowlisted variables from `environment`.
    pub fn new(environment: impl IntoIterator<Item = (OsString, OsString)>) -> Self {
        Self {
            inherited: environment
                .into_iter()
                .filter(|(name, _)| INHERITED.iter().any(|allowed| name == allowed))
                .collect(),
        }
    }
}

impl Runner for ProcessRunner {
    fn run<'a>(&'a mut self, step: &'a Step) -> StepFuture<'a> {
        Box::pin(async move {
            let mut command = crate::command::Command::new(&step.program);
            command
                .args(&step.args)
                .current_dir(&step.dir)
                .env_clear()
                .envs(self.inherited.iter().map(|(name, value)| (name, value)))
                .envs(step.env.iter().map(|(name, value)| (name, value)))
                .kill_on_drop(true);
            crate::command::bounded_output(&mut command, step.timeout, OUTPUT_LIMIT)
                .await
                .with_context(|| format!("run {} ({})", step.label, step.program.display()))
        })
    }

    fn path(&self) -> Option<&OsStr> {
        self.inherited
            .iter()
            .find(|(name, _)| name == "PATH")
            .map(|(_, value)| value.as_os_str())
    }
}

/// Refuse before fetching or compiling anything when the host or mode cannot
/// produce authoritative bytes for this recipe.
fn preflight(options: &BuildOptions) -> Result<bool> {
    ensure!(
        !options.offline,
        "bundle build needs the Go module proxy and ICU release download; unset KURU_DOLT_BUNDLE_OFFLINE"
    );
    let authoritative = options.host.name() == BUILD_HOST;
    ensure!(
        authoritative || options.host_override,
        "bundle build runs only on {BUILD_HOST}, not {}; KURU_BUNDLE_BUILD_HOST_OVERRIDE=1 allows non-authoritative local iteration",
        options.host.name()
    );
    if !authoritative {
        eprintln!(
            "warning: KURU_BUNDLE_BUILD_HOST_OVERRIDE=1 on {}: this build is for local iteration only; its bytes are not authoritative pins",
            options.host.name()
        );
    }
    ensure!(
        options.work_dir.is_absolute() && options.output.is_absolute(),
        "bundle build work and output directories must be absolute"
    );
    ensure!(
        !options.work_dir.starts_with(&options.output)
            && !options.output.starts_with(&options.work_dir),
        "bundle build work and output directories must be separate"
    );
    ensure!(options.jobs > 0, "bundle build needs at least one job");
    Ok(authoritative)
}

/// Build the target's pinned engine archive from source.
pub async fn build(options: &BuildOptions) -> Result<BuildReport> {
    let mut runner = ProcessRunner::new(std::env::vars_os());
    let client = reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(30))
        .timeout(ICU_DOWNLOAD)
        .build()?;
    build_with(options, &mut runner, &client, None).await
}

/// The complete build. `icu_url` replaces the pinned ICU URL only for local
/// HTTP fixtures; the size and digest pins still apply.
pub(crate) async fn build_with(
    options: &BuildOptions,
    runner: &mut dyn Runner,
    client: &reqwest::Client,
    icu_url: Option<&str>,
) -> Result<BuildReport> {
    let authoritative = preflight(options)?;
    let manifest = load_manifest(&options.manifest)?;
    let asset = manifest.select(&options.target)?;
    ensure!(
        asset.provenance == Provenance::Built,
        "{} uses the upstream Dolt archive; prepare it with bundle:prepare instead of building it",
        options.target
    );
    let recipe = asset
        .build
        .as_ref()
        .context("built asset has no build recipe")?;
    ensure!(
        options.print_pins || asset.archive_pinned(),
        "{}; pass --print-pins to report the observed pins",
        super::unpinned_message(&options.target)
    );
    ensure!(
        authoritative || options.print_pins,
        "a non-authoritative host cannot verify pins; pass --print-pins to inspect its bytes"
    );

    let parent = options
        .work_dir
        .parent()
        .context("bundle build work directory needs a parent")?;
    let name = options
        .work_dir
        .file_name()
        .context("bundle build work directory needs a name")?;
    let work = CheckedDirectory::ensure_private(parent)
        .context("open bundle build work parent")?
        .create_private_directory(name)
        .context("bundle build needs a fresh private work directory")?;
    let work = work.path().to_owned();
    for directory in ["gomodcache", "gocache", "gopath", "tmp", "fetch"] {
        fs::create_dir(work.join(directory))?;
    }
    let output = CheckedDirectory::ensure_private(&options.output)
        .context("open private bundle build output directory")?;

    let go = &recipe.toolchain.go;
    let llvm = &recipe.toolchain.llvm_mingw;
    let base_env = vec![
        env("GOTOOLCHAIN", "local"),
        env("GOENV", "off"),
        env("GOWORK", "off"),
        env("GOTELEMETRY", "off"),
        env("GOPROXY", "https://proxy.golang.org"),
        env("GOSUMDB", "sum.golang.org"),
        env("GOMODCACHE", work.join("gomodcache")),
        env("GOCACHE", work.join("gocache")),
        env("GOPATH", work.join("gopath")),
        env("TMPDIR", work.join("tmp")),
        env("LC_ALL", "C"),
        env("TZ", "UTC"),
    ];
    let step = |label, program: &str, args: Vec<OsString>, dir: &Path, extra, timeout| Step {
        label,
        program: program.into(),
        args,
        dir: dir.to_owned(),
        env: [base_env.clone(), extra].concat(),
        timeout,
    };

    // 1. Exact toolchains.
    let reported = stdout(
        runner,
        &step(
            "go version",
            "go",
            args(["version"]),
            &work,
            Vec::new(),
            QUICK,
        ),
    )
    .await?;
    let go_version = reported.trim().to_owned();
    let expected_go = if authoritative {
        go_version == format!("go version {} linux/amd64", go.version)
    } else {
        go_version.starts_with(&format!("go version {} ", go.version))
    };
    ensure!(
        expected_go,
        "Go toolchain reports {go_version:?}; the recipe pins {}",
        go.version
    );
    let clang = format!("{CROSS_HOST}-clang");
    let reported = stdout(
        runner,
        &step(
            "clang version",
            &clang,
            args(["--version"]),
            &work,
            Vec::new(),
            QUICK,
        ),
    )
    .await?;
    let clang_version = reported
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_owned();
    ensure!(
        clang_version
            .split_whitespace()
            .collect::<Vec<_>>()
            .windows(3)
            .any(|words| words[0] == "clang"
                && words[1] == "version"
                && words[2] == llvm.clang_version),
        "llvm-mingw clang reports {clang_version:?}; the recipe pins clang {}",
        llvm.clang_version
    );
    let toolchain_root = toolchain_root(runner.path(), &clang)?;
    println!("llvm-mingw root: {}", toolchain_root.display());
    println!("{go_version}");
    println!("{clang_version}");

    // 2. Dolt source from the module proxy, checked by its go.sum hash.
    let dolt = &recipe.sources.dolt;
    let module = format!("{}@{}", dolt.module, dolt.version);
    let reported = stdout(
        runner,
        &step(
            "go mod download",
            "go",
            args(["mod", "download", "-json", module.as_str()]),
            &work.join("fetch"),
            vec![env("GOFLAGS", "-mod=mod -modcacherw")],
            FETCH,
        ),
    )
    .await?;
    let downloaded: ModuleDownload =
        serde_json::from_str(&reported).context("parse go mod download output")?;
    ensure!(
        downloaded.error.is_none(),
        "go mod download failed: {}",
        downloaded.error.unwrap_or_default()
    );
    ensure!(
        downloaded.path == dolt.module && downloaded.version == dolt.version,
        "module proxy returned a different Dolt module"
    );
    ensure!(
        downloaded.sum == dolt.sum,
        "Dolt module checksum {} does not match the pinned {}",
        downloaded.sum,
        dolt.sum
    );
    let module_dir = PathBuf::from(&downloaded.dir);
    ensure!(
        module_dir.is_absolute() && module_dir.starts_with(work.join("gomodcache")),
        "downloaded Dolt module is outside the private module cache"
    );
    let source = work.join("src");
    copy_tree(&module_dir, &source)?;
    let version_file = read_bounded(&source.join("cmd/dolt/doltversion/version.go"), 64 * 1024)?;
    ensure!(
        String::from_utf8_lossy(&version_file)
            .contains(&format!("Version = \"{}\"", manifest.version)),
        "Dolt module does not declare version {}",
        manifest.version
    );
    let license = read_bounded(&source.join("Godeps/LICENSES"), MAX_EXPANDED)?;
    ensure!(
        license.len() as u64 == asset.license_bytes
            && crate::archive::digest(&license) == asset.license_sha256,
        "Dolt Godeps/LICENSES does not match the pinned LICENSES"
    );

    // 3. ICU sources through the bounded, digest-verified download client.
    let icu = &recipe.sources.icu;
    let tarball = work.join("fetch/icu4c-sources.tgz");
    {
        let mut file = fs::File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&tarball)?;
        let pinned = Asset {
            target: format!("ICU {} sources", icu.version),
            url: icu_url.unwrap_or(&icu.url).to_owned(),
            built: false,
            compressed_bytes: icu.bytes,
            archive_sha256: icu.sha256.clone(),
        };
        download(client, &pinned, &mut file, ICU_DOWNLOAD, &RETRY_DELAYS)
            .await
            .context("fetch pinned ICU sources")?;
        file.sync_all()?;
    }
    let icu_tree = work.join("icu-src");
    extract_sources(&read_bounded(&tarball, icu.bytes)?, &icu_tree)?;
    let icu_source = icu_tree.join("icu/source");

    // 4. Static ICU: host tools, then the cross build with stub data.
    let prefix_map = format!("-ffile-prefix-map={}={PREFIX_MAP}", work.display());
    let host_build = work.join("icu-host");
    let cross_build = work.join("icu-cross");
    for directory in [&host_build, &cross_build] {
        fs::create_dir(directory)?;
    }
    let make_args = || args([format!("-j{}", options.jobs).as_str(), "SHELL=/bin/bash"]);
    let host_platform = if options.host.os == "macos" {
        "MacOSX"
    } else {
        "Linux"
    };
    checked(
        runner,
        &step(
            "configure host ICU",
            "sh",
            vec![
                icu_source.join("runConfigureICU").into(),
                host_platform.into(),
                "--disable-tests".into(),
                "--disable-samples".into(),
            ],
            &host_build,
            Vec::new(),
            COMPILE,
        ),
    )
    .await?;
    checked(
        runner,
        &step(
            "build host ICU",
            "make",
            make_args(),
            &host_build,
            Vec::new(),
            COMPILE,
        ),
    )
    .await?;
    checked(
        runner,
        &step(
            "configure cross ICU",
            "sh",
            vec![
                icu_source.join("configure").into(),
                format!("--host={CROSS_HOST}").into(),
                format!("--with-cross-build={}", host_build.display()).into(),
                "--enable-static".into(),
                "--disable-shared".into(),
                "--with-data-packaging=static".into(),
                "--disable-tests".into(),
                "--disable-samples".into(),
                "--disable-extras".into(),
                "--disable-tools".into(),
                format!("CC={CROSS_HOST}-clang").into(),
                format!("CXX={CROSS_HOST}-clang++").into(),
                format!("CFLAGS=-O2 {prefix_map}").into(),
                format!("CXXFLAGS=-O2 -std=c++17 {prefix_map}").into(),
            ],
            &cross_build,
            Vec::new(),
            COMPILE,
        ),
    )
    .await?;
    checked(
        runner,
        &step(
            "build cross ICU",
            "make",
            make_args(),
            &cross_build,
            Vec::new(),
            COMPILE,
        ),
    )
    .await?;
    let link = work.join("icu-link");
    fs::create_dir(&link)?;
    for (from, name) in [
        (cross_build.join("lib/libsicuin.a"), "libsicuin.a"),
        (cross_build.join("lib/libsicuuc.a"), "libsicuuc.a"),
        (cross_build.join("stubdata/libsicudt.a"), "libsicudt.a"),
    ] {
        let bytes = read_bounded(&from, MAX_EXPANDED)?;
        ensure!(
            name != "libsicudt.a" || bytes.len() as u64 <= MAX_STUB_DATA,
            "ICU data library is not the stub; full ICU data exceeds the engine budget"
        );
        fs::write(link.join(name), bytes)?;
    }

    // 5. Dolt with cgo against static ICU.
    let executable_path = work.join("dolt.exe");
    checked(
        runner,
        &step(
            "go build",
            "go",
            vec![
                "build".into(),
                "-trimpath".into(),
                "-buildvcs=false".into(),
                "-tags".into(),
                recipe.tags.join(",").into(),
                "-ldflags=-s -w -buildid=".into(),
                "-o".into(),
                executable_path.clone().into(),
                "./cmd/dolt".into(),
            ],
            &source,
            vec![
                env("GOFLAGS", "-mod=readonly -modcacherw"),
                env("CGO_ENABLED", "1"),
                env("GOOS", &recipe.goos),
                env("GOARCH", &recipe.goarch),
                env("CC", format!("{CROSS_HOST}-clang")),
                env("CXX", format!("{CROSS_HOST}-clang++")),
                env(
                    "CGO_CPPFLAGS",
                    format!(
                        "-I{} -I{}",
                        icu_source.join("common").display(),
                        icu_source.join("i18n").display()
                    ),
                ),
                env("CGO_CFLAGS", format!("-O2 -g {prefix_map}")),
                env(
                    "CGO_CXXFLAGS",
                    format!("-O2 -g -include cstdlib {prefix_map}"),
                ),
                env("CGO_LDFLAGS", format!("-L{} -static", link.display())),
            ],
            COMPILE,
        ),
    )
    .await?;
    let executable = read_bounded(&executable_path, MAX_EXPANDED)?;
    let imports = check_pe(&executable)?;
    println!("dolt.exe imports: {}", imports.join(", "));

    // 6. Notices from the pinned ICU sources and toolchain.
    let mut notices = Vec::new();
    for notice in asset.notices() {
        ensure!(
            relative_path(&notice.path),
            "invalid notice path {}",
            notice.path
        );
        let root = match notice.from {
            NoticeSource::Icu => &icu_tree,
            NoticeSource::LlvmMingw => &toolchain_root,
        };
        let bytes = read_bounded(&root.join(&notice.path), MAX_NOTICE)
            .with_context(|| format!("read notice {} from {}", notice.name, notice.path))?;
        ensure!(!bytes.is_empty(), "notice {} is empty", notice.name);
        notices.push((notice.name.clone(), bytes));
    }

    // 7. Package in the manifest layout.
    let stem = &asset.stem;
    let names = [
        format!("{stem}/"),
        format!("{stem}/bin/"),
        format!("{stem}/bin/{}", asset.executable_name),
        format!("{stem}/LICENSES"),
    ];
    let notice_names: Vec<_> = notices
        .iter()
        .map(|(name, _)| format!("{stem}/{name}"))
        .collect();
    let mut members = vec![
        WriteMember {
            name: &names[0],
            kind: MemberKind::Directory,
            bytes: &[],
            executable: false,
        },
        WriteMember {
            name: &names[1],
            kind: MemberKind::Directory,
            bytes: &[],
            executable: false,
        },
        WriteMember {
            name: &names[2],
            kind: MemberKind::File,
            bytes: &executable,
            executable: true,
        },
        WriteMember {
            name: &names[3],
            kind: MemberKind::File,
            bytes: &license,
            executable: false,
        },
    ];
    members.extend(
        notice_names
            .iter()
            .zip(&notices)
            .map(|(name, (_, bytes))| WriteMember {
                name,
                kind: MemberKind::File,
                bytes,
                executable: false,
            }),
    );
    let archive = kuru_archive::zip::write(
        &members,
        Limits {
            max_compressed_bytes: MAX_ARCHIVE,
            max_expanded_bytes: MAX_EXPANDED,
            allow_ntfs_timestamps: false,
        },
    )
    .context("package the built engine within the manifest bounds")?;
    let pins = ObservedPins {
        target: asset.target.clone(),
        authoritative,
        go_version,
        clang_version,
        compressed_bytes: archive.len() as u64,
        archive_sha256: crate::archive::digest(&archive),
        expanded_bytes: members.iter().map(|member| member.bytes.len() as u64).sum(),
        executable_bytes: executable.len() as u64,
        executable_sha256: crate::archive::digest(&executable),
        license_bytes: license.len() as u64,
        license_sha256: crate::archive::digest(&license),
        notices: notices
            .iter()
            .map(|(name, bytes)| ObservedNotice {
                name: name.clone(),
                bytes: bytes.len() as u64,
                sha256: crate::archive::digest(bytes),
            })
            .collect(),
        imports,
    };

    let archive_name = format!("{stem}.zip");
    let mut file = output
        .create_new(OsStr::new(&archive_name))
        .context("bundle build output must not already contain the archive")?;
    file.write_all(&archive)?;
    file.sync_all()?;
    let mut rendered = serde_json::to_vec_pretty(&pins)?;
    rendered.push(b'\n');
    let mut file = output
        .create_new(OsStr::new("pins.json"))
        .context("bundle build output must not already contain pins.json")?;
    file.write_all(&rendered)?;
    file.sync_all()?;
    let report = BuildReport {
        archive: output.path().join(archive_name),
        pins_file: output.path().join("pins.json"),
        verified: authoritative && asset.archive_pinned(),
        pins,
    };
    // Verification runs after the observed pins are durable, so a mismatch
    // still leaves the evidence needed to diagnose it. Another host's bytes
    // are expected to differ and are never compared or committed.
    let mismatches = if authoritative {
        mismatches(asset, &report.pins)
    } else {
        Vec::new()
    };
    ensure!(
        mismatches.is_empty(),
        "built engine differs from the committed pins:\n{}",
        mismatches.join("\n")
    );
    Ok(report)
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ModuleDownload {
    path: String,
    version: String,
    #[serde(default)]
    sum: String,
    #[serde(default)]
    dir: String,
    #[serde(default)]
    error: Option<String>,
}

fn env(name: &str, value: impl Into<OsString>) -> (OsString, OsString) {
    (name.into(), value.into())
}

fn args<const N: usize>(values: [&str; N]) -> Vec<OsString> {
    values.into_iter().map(OsString::from).collect()
}

fn tail(bytes: &[u8]) -> String {
    let start = bytes.len().saturating_sub(16 * 1024);
    String::from_utf8_lossy(&bytes[start..]).into_owned()
}

async fn checked(runner: &mut dyn Runner, step: &Step) -> Result<Output> {
    println!("==> {}", step.label);
    let output = runner.run(step).await?;
    ensure!(
        output.status.success(),
        "{} failed with {}\nstdout (tail):\n{}\nstderr (tail):\n{}",
        step.label,
        output.status,
        tail(&output.stdout),
        tail(&output.stderr)
    );
    Ok(output)
}

async fn stdout(runner: &mut dyn Runner, step: &Step) -> Result<String> {
    let output = checked(runner, step).await?;
    String::from_utf8(output.stdout).with_context(|| format!("{} printed non-UTF-8", step.label))
}

/// The llvm-mingw root is the parent of the `bin` directory that provides the
/// cross compiler on the child `PATH`.
fn toolchain_root(path: Option<&OsStr>, compiler: &str) -> Result<PathBuf> {
    let path = path.context("PATH is required to locate llvm-mingw")?;
    std::env::split_paths(path)
        .filter(|directory| directory.is_absolute())
        .find(|directory| directory.join(compiler).is_file())
        .and_then(|bin| bin.parent().map(Path::to_path_buf))
        .with_context(|| format!("{compiler} is not on PATH; run the bundle:build mise task"))
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("inspect {}", path.display()))?;
    ensure!(
        metadata.file_type().is_file() && metadata.len() <= limit,
        "{} must be a bounded regular file",
        path.display()
    );
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "{} changed while reading",
        path.display()
    );
    Ok(bytes)
}

/// Copy the read-only module cache tree into a writable private source tree,
/// accepting only directories and regular files.
fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    let mut pending = vec![(from.to_owned(), to.to_owned())];
    let (mut entries, mut bytes) = (0_usize, 0_u64);
    while let Some((source, destination)) = pending.pop() {
        fs::create_dir(&destination)?;
        for entry in fs::read_dir(&source)? {
            let entry = entry?;
            entries += 1;
            ensure!(
                entries <= MAX_SOURCE_ENTRIES,
                "Dolt module has too many entries"
            );
            let kind = entry.file_type()?;
            let target = destination.join(entry.file_name());
            if kind.is_dir() {
                pending.push((entry.path(), target));
            } else if kind.is_file() {
                bytes += fs::copy(entry.path(), &target)?;
                ensure!(bytes <= MAX_SOURCE_TREE, "Dolt module exceeds its budget");
                writable(&target)?;
            } else {
                bail!("Dolt module contains a link or special file");
            }
        }
    }
    Ok(())
}

/// The module cache is read-only; the copied source tree stays owner-writable.
fn writable(path: &Path) -> Result<()> {
    let mut permissions = fs::metadata(path)?.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(permissions.mode() | 0o200);
    }
    #[cfg(not(unix))]
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

/// Extract the pinned source tarball, accepting only regular files and
/// directories with normal relative paths.
fn extract_sources(tarball: &[u8], destination: &Path) -> Result<()> {
    fs::create_dir(destination)?;
    let decoder = flate2::read::GzDecoder::new(tarball).take(MAX_SOURCE_TREE + 1);
    let mut archive = tar::Archive::new(decoder);
    let mut entries = 0_usize;
    for entry in archive.entries()? {
        let mut entry = entry?;
        entries += 1;
        ensure!(
            entries <= MAX_SOURCE_ENTRIES,
            "source archive has too many entries"
        );
        let kind = entry.header().entry_type();
        ensure!(
            kind.is_file() || kind.is_dir(),
            "source archive contains a link or special entry"
        );
        let path = entry.path()?.into_owned();
        ensure!(
            path.components()
                .all(|component| matches!(component, Component::Normal(_))),
            "source archive contains an unsafe path"
        );
        ensure!(
            entry.unpack_in(destination)?,
            "source archive entry escaped its destination"
        );
    }
    Ok(())
}

fn u16_at(bytes: &[u8], offset: usize) -> Result<u16> {
    let slice = bytes
        .get(offset..offset + 2)
        .context("truncated PE image")?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32> {
    let slice = bytes
        .get(offset..offset + 4)
        .context("truncated PE image")?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

/// Require a Windows ARM64 PE32+ image whose ordinary and delay-load imports
/// are all operating-system libraries, and return their sorted names.
pub(crate) fn check_pe(image: &[u8]) -> Result<Vec<String>> {
    ensure!(image.starts_with(b"MZ"), "built engine is not a PE image");
    let header = u32_at(image, 0x3c)? as usize;
    ensure!(
        image.get(header..header + 4) == Some(b"PE\0\0"),
        "built engine has no PE signature"
    );
    let coff = header + 4;
    let machine = u16_at(image, coff)?;
    ensure!(
        machine == IMAGE_FILE_MACHINE_ARM64,
        "built engine targets PE machine {machine:#06x}, not ARM64 (0xaa64)"
    );
    let sections = u16_at(image, coff + 2)? as usize;
    let optional_size = u16_at(image, coff + 16)? as usize;
    let optional = coff + 20;
    ensure!(
        u16_at(image, optional)? == 0x20b,
        "built engine is not a PE32+ image"
    );
    let directories = u32_at(image, optional + 108)? as usize;
    let section_table = optional + optional_size;
    let rva = |address: u32| -> Result<usize> {
        for index in 0..sections {
            let section = section_table + index * 40;
            let size = u32_at(image, section + 8)?.max(u32_at(image, section + 16)?);
            let start = u32_at(image, section + 12)?;
            if address >= start && address - start < size {
                return Ok((address - start) as usize + u32_at(image, section + 20)? as usize);
            }
        }
        bail!("PE address {address:#x} is outside every section")
    };
    let name = |address: u32| -> Result<String> {
        let start = rva(address)?;
        let bytes = image.get(start..).context("truncated PE import name")?;
        let end = bytes
            .iter()
            .take(256)
            .position(|byte| *byte == 0)
            .context("unterminated PE import name")?;
        Ok(String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase())
    };
    let mut libraries = Vec::new();
    // Data directory 1 is the import table and 13 the delay-load table.
    for (directory, stride, name_offset) in [(1_usize, 20_usize, 12_usize), (13, 32, 4)] {
        if directory >= directories {
            continue;
        }
        let address = u32_at(image, optional + 112 + directory * 8)?;
        if address == 0 {
            continue;
        }
        let table = rva(address)?;
        for index in 0..1024 {
            let descriptor = table + index * stride;
            let record = image
                .get(descriptor..descriptor + stride)
                .context("truncated PE import table")?;
            if record.iter().all(|byte| *byte == 0) {
                break;
            }
            libraries.push(name(u32_at(image, descriptor + name_offset)?)?);
        }
    }
    libraries.sort();
    libraries.dedup();
    ensure!(!libraries.is_empty(), "built engine has no DLL imports");
    for library in &libraries {
        ensure!(
            SYSTEM_LIBRARIES.contains(&library.as_str()) || api_set(library),
            "built engine requires an unapproved external DLL: {library}"
        );
    }
    Ok(libraries)
}

/// `^(api|ext)-ms-win-[a-z0-9-]+-l[0-9]+-[0-9]+-[0-9]+\.dll$`
fn api_set(library: &str) -> bool {
    let Some(stem) = library
        .strip_prefix("api-ms-win-")
        .or_else(|| library.strip_prefix("ext-ms-win-"))
        .and_then(|rest| rest.strip_suffix(".dll"))
    else {
        return false;
    };
    let parts: Vec<_> = stem.rsplitn(4, '-').collect();
    parts.len() == 4
        && parts[..2]
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        && parts[2].strip_prefix('l').is_some_and(|level| {
            !level.is_empty() && level.bytes().all(|byte| byte.is_ascii_digit())
        })
        && !parts[3].is_empty()
        && parts[3]
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

/// Compare observed pins with every pinned manifest value.
fn mismatches(asset: &ManifestAsset, observed: &ObservedPins) -> Vec<String> {
    let mut differences = Vec::new();
    let mut compare = |label: &str, pinned: String, actual: String| {
        if pinned != actual {
            differences.push(format!("{label}: pinned {pinned}, observed {actual}"));
        }
    };
    if asset.archive_pinned() {
        compare(
            "archive_sha256",
            asset.archive_sha256.clone(),
            observed.archive_sha256.clone(),
        );
        compare(
            "compressed_bytes",
            asset.compressed_bytes.unwrap_or_default().to_string(),
            observed.compressed_bytes.to_string(),
        );
        compare(
            "expanded_bytes",
            asset.expanded_bytes.unwrap_or_default().to_string(),
            observed.expanded_bytes.to_string(),
        );
        compare(
            "executable_sha256",
            asset.executable_sha256.clone(),
            observed.executable_sha256.clone(),
        );
        compare(
            "executable_bytes",
            asset.executable_bytes.unwrap_or_default().to_string(),
            observed.executable_bytes.to_string(),
        );
    }
    for (pinned, actual) in asset.notices().iter().zip(&observed.notices) {
        if pinned.sha256 != UNPINNED {
            compare(
                &format!("{} sha256", pinned.name),
                pinned.sha256.clone(),
                actual.sha256.clone(),
            );
            compare(
                &format!("{} bytes", pinned.name),
                pinned.bytes.unwrap_or_default().to_string(),
                actual.bytes.to_string(),
            );
        }
    }
    differences
}

/// Render the one-line summary CI prints and the manifest fields to commit.
pub fn summary(report: &BuildReport) -> String {
    let status = if report.verified {
        "verified against the committed pins".to_owned()
    } else {
        "unpinned: pin verification not yet possible".to_owned()
    };
    let authority = if report.pins.authoritative {
        String::new()
    } else {
        " (non-authoritative host override)".to_owned()
    };
    format!(
        "archive: {}\narchive sha256: {}\narchive bytes: {}\npins: {}\nstatus: {status}{authority}",
        report.archive.display(),
        report.pins.archive_sha256,
        report.pins.compressed_bytes,
        report.pins_file.display(),
    )
}

#[cfg(test)]
#[path = "build_tests.rs"]
mod tests;
