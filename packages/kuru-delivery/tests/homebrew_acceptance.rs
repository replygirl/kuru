//! Native acceptance for staged releases and ordinary native installation CI.
//! Takes RELEASE_VERSION and absolute KURU_HOMEBREW_RELEASE_DIR, or an absolute
//! KURU_HOMEBREW_CANDIDATE_BINARY for an isolated simulated release. GITHUB_TOKEN
//! authenticates only previous-release metadata, never Homebrew or Kuru children.
#![cfg(all(unix, feature = "tooling"))]

use anyhow::{Context, Result, ensure};
use axum::{
    Router,
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use kuru_delivery::{
    archive,
    command::{self, Command},
    homebrew, published, release, shell_support, targets,
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Output,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

#[path = "support/fixture_git.rs"]
mod fixture_git;
#[path = "support/launch_budget.rs"]
mod launch_budget;

const BREW_BOUND: Duration = Duration::from_secs(300);
const KURU_BOUND: Duration = Duration::from_secs(100);
const OUTPUT_LIMIT: usize = 4 * 1024 * 1024;

struct Assets {
    files: BTreeMap<String, Vec<u8>>,
    requests: AtomicUsize,
}
async fn serve(State(assets): State<Arc<Assets>>, request: Request) -> Response {
    assert!(
        !request.headers().contains_key("authorization"),
        "asset server received credentials"
    );
    assets.requests.fetch_add(1, Ordering::Relaxed);
    match assets.files.get(request.uri().path()) {
        Some(bytes) => bytes.clone().into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn brew(args: &[&str], root: &Path) -> Result<Output> {
    eprintln!("Homebrew acceptance: brew {args:?}");
    let mut child = Command::new("brew");
    child
        .args(args)
        .env("HOMEBREW_NO_AUTO_UPDATE", "1")
        .env("HOMEBREW_NO_ANALYTICS", "1")
        .env("HOMEBREW_NO_INSTALL_CLEANUP", "1")
        .env("HOMEBREW_NO_INSTALLED_DEPENDENTS_CHECK", "1")
        .env_remove("HOMEBREW_NO_INSTALL_FROM_API")
        .env("HOMEBREW_CACHE", root.join("brew-cache"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_COUNT", "3")
        .env("GIT_CONFIG_KEY_0", "credential.helper")
        .env("GIT_CONFIG_VALUE_0", "")
        .env("GIT_CONFIG_KEY_1", "core.hooksPath")
        .env("GIT_CONFIG_VALUE_1", root.join("empty-hooks"))
        .env("GIT_CONFIG_KEY_2", "core.fsmonitor")
        .env("GIT_CONFIG_VALUE_2", "false");
    for name in [
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "HOMEBREW_GITHUB_API_TOKEN",
        "GIT_ASKPASS",
        "SSH_ASKPASS",
        "OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
    ] {
        child.env_remove(name);
    }
    let output = command::bounded_output(&mut child, BREW_BOUND, OUTPUT_LIMIT).await?;
    ensure!(
        output.status.success(),
        "brew {args:?} failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(output)
}

fn text(output: Output) -> Result<String> {
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

/// Only native acceptance substitutes these exact fixture URLs. Production
/// generation has no configurable asset host or insecure transport option.
fn local_formula(formula: &str, base: &str) -> Result<String> {
    let url = url::Url::parse(base)?;
    ensure!(
        url.scheme() == "http"
            && url.host_str() == Some("127.0.0.1")
            && url.port().is_some()
            && url.path() == "/"
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "acceptance URL must be a literal loopback listener"
    );
    Ok(formula.replace("https://github.com/replygirl/kuru/releases/download/", base))
}

fn payload_documents(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
    let mut documents = BTreeMap::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let name = entry.path()?.to_string_lossy().into_owned();
        if name == "LICENSE" || name == "README.md" {
            let mut content = Vec::new();
            entry.read_to_end(&mut content)?;
            documents.insert(name, content);
        }
    }
    ensure!(documents.len() == 2, "verified archive lacks its documents");
    Ok(documents)
}

fn exact_install(
    keg: &Path,
    binary: &[u8],
    support: Option<&shell_support::Files>,
    core: &[u8],
) -> Result<()> {
    ensure!(
        fs::read(keg.join("bin/kuru"))? == binary,
        "Homebrew changed the release executable (including its signature)"
    );
    for (name, expected) in payload_documents(core)? {
        ensure!(
            fs::read(keg.join("share/kuru").join(name))? == expected,
            "Homebrew changed a release document"
        );
    }
    if let Some(files) = support {
        for (source, installed) in [
            ("completions/kuru.bash", "etc/bash_completion.d/kuru"),
            ("completions/_kuru", "share/zsh/site-functions/_kuru"),
            (
                "completions/kuru.fish",
                "share/fish/vendor_completions.d/kuru.fish",
            ),
            ("completions/kuru.ps1", "share/pwsh/completions/kuru.ps1"),
            ("man/kuru.1", "share/man/man1/kuru.1"),
        ] {
            ensure!(
                fs::read(keg.join(installed))?.as_slice()
                    == files.get(source).context("support file missing")?,
                "Homebrew changed paired support {source}"
            );
        }
    }
    Ok(())
}

struct Offline {
    root: PathBuf,
    binary: PathBuf,
    cellar: PathBuf,
}

async fn wait_for_quiescence(root: &Path) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        let processes = sysinfo::ProcessRefreshKind::nothing()
            .with_cmd(sysinfo::UpdateKind::Always)
            .with_exe(sysinfo::UpdateKind::Always);
        let system = sysinfo::System::new_with_specifics(
            sysinfo::RefreshKind::nothing().with_processes(processes),
        );
        let live = system
            .processes()
            .iter()
            .filter_map(|(pid, process)| {
                let belongs = process.exe().is_some_and(|path| path.starts_with(root))
                    || process.cmd().iter().any(|arg| {
                        arg.to_string_lossy()
                            .contains(root.to_string_lossy().as_ref())
                    });
                belongs.then_some(pid.to_string())
            })
            .collect::<Vec<_>>();
        if live.is_empty() {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "installed memory processes did not retire: {live:?}; private state retained at {}",
            root.display()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

impl Offline {
    fn new(root: &Path, binary: PathBuf, cellar: PathBuf) -> Result<Self> {
        for name in ["home", "config/kuru", "empty-path", "tmp", "project"] {
            fs::create_dir_all(root.join(name))?;
        }
        let cache = root.join("engine-cache");
        fs::write(
            root.join("config/kuru/config.toml"),
            format!(
                "[memory]\noffline = true\ncache_dir = {}\nservice_idle_timeout_secs = 0\n",
                toml::Value::String(cache.to_string_lossy().into_owned())
            ),
        )?;
        ensure!(
            !cache.exists() && !root.join("data").exists(),
            "offline acceptance must be cold"
        );
        Ok(Self {
            root: root.to_owned(),
            binary,
            cellar,
        })
    }
    fn command(&self) -> Command {
        let mut child = Command::new(&self.binary);
        child
            .env_clear()
            .env("HOME", self.root.join("home"))
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_CACHE_HOME", self.root.join("cache"))
            .env("XDG_DATA_HOME", self.root.join("xdg-data"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("TMPDIR", self.root.join("tmp"))
            .env("PATH", self.root.join("empty-path"))
            .env("HOMEBREW_CELLAR", &self.cellar)
            .current_dir(self.root.join("project"))
            .arg("--data-dir")
            .arg(self.root.join("data"))
            .args(["--provider", "demo", "--mode", "freudian", "--no-dream"]);
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            child.env("LLVM_PROFILE_FILE", profile);
        }
        child
    }
    async fn run(&self, args: &[&str]) -> Result<Value> {
        let output =
            command::bounded_output(self.command().args(args), KURU_BOUND, OUTPUT_LIMIT).await?;
        ensure!(
            output.status.success()
                && output.stdout.len() <= OUTPUT_LIMIT
                && output.stderr.len() <= OUTPUT_LIMIT,
            "installed Kuru {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).context("installed Kuru did not return JSON")
    }
    async fn quiescent(&self) -> Result<()> {
        wait_for_quiescence(&self.root).await
    }
    async fn conversation(&self) -> Result<String> {
        let first = self
            .run(&[
                "run",
                "Remember the Homebrew offline marker amber-731",
                "--json",
            ])
            .await?;
        let session = first["session"].as_str().context("first session missing")?;
        ensure!(
            !first["text"]
                .as_str()
                .context("response missing")?
                .is_empty(),
            "offline response is empty"
        );
        let status = self.run(&["memory", "status"]).await?;
        ensure!(
            status["engine"] == "dolt",
            "offline memory did not use bundled full Dolt"
        );
        self.quiescent().await?;
        let second = self
            .run(&[
                "--resume",
                session,
                "run",
                "Continue the saved Homebrew conversation",
                "--json",
            ])
            .await?;
        ensure!(
            second["session"] == session,
            "offline restart lost the durable session"
        );
        let after = self.run(&["memory", "status"]).await?;
        ensure!(
            status["revision"] != after["revision"],
            "offline restart did not persist a new revision"
        );
        let sessions = self.run(&["sessions"]).await?;
        ensure!(
            sessions
                .as_array()
                .context("session list")?
                .iter()
                .any(|entry| entry["id"] == session),
            "saved session missing after offline reopen"
        );
        self.quiescent().await?;
        Ok(session.to_owned())
    }
}

async fn commit_formula(
    git: &fixture_git::FixtureGit,
    repository: &Path,
    formula: &str,
) -> Result<()> {
    fs::write(repository.join("Formula/kuru.rb"), formula)?;
    for args in [
        ["add", "Formula/kuru.rb"].as_slice(),
        [
            "-c",
            "user.name=Kuru acceptance",
            "-c",
            "user.email=acceptance@example.invalid",
            "commit",
            "-m",
            "chore: accepted fixture formula",
        ]
        .as_slice(),
    ] {
        let output = git.git(repository, args).await;
        ensure!(
            output.status.success(),
            "fixture Git failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

fn package_simulated_release(
    binary: &Path,
    generated: &Path,
    version: release::Version,
    output: &Path,
) -> Result<()> {
    fs::create_dir(output)?;
    for target in targets::CATALOG {
        archive::package(binary, target.triple, &version.to_string(), output)?;
        shell_support::package(generated, target.triple, &version.to_string(), output)?;
    }
    let notes = output
        .parent()
        .context("simulated release has no parent")?
        .join("SIMULATED_NOTES.md");
    fs::write(
        &notes,
        "Native Homebrew acceptance fixture only. Foreign target names model archive shape; their executable bytes are the host binary and establish no foreign architecture or signing acceptance.\n",
    )?;
    release::assemble(output, version, &notes)?;
    Ok(())
}

async fn simulated_release(root: &Path, binary: PathBuf) -> Result<(PathBuf, release::Version)> {
    ensure!(
        binary.is_absolute() && binary.is_file(),
        "KURU_HOMEBREW_CANDIDATE_BINARY must name an absolute native release executable"
    );
    let generator = root.join("pure-generator");
    for name in ["home", "config", "empty-path", "tmp", "project"] {
        fs::create_dir_all(generator.join(name))?;
    }
    let launch = || {
        let mut child = Command::new(&binary);
        child
            .env_clear()
            .env("HOME", generator.join("home"))
            .env("XDG_CONFIG_HOME", generator.join("config"))
            .env("XDG_DATA_HOME", generator.join("data"))
            .env("XDG_CACHE_HOME", generator.join("cache"))
            .env("XDG_STATE_HOME", generator.join("state"))
            .env("TMPDIR", generator.join("tmp"))
            .env("PATH", generator.join("empty-path"))
            .current_dir(generator.join("project"));
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            child.env("LLVM_PROFILE_FILE", profile);
        }
        child
    };
    let report = command::bounded_output(
        launch().arg("--version"),
        Duration::from_secs(20),
        OUTPUT_LIMIT,
    )
    .await?;
    ensure!(
        report.status.success(),
        "native fixture binary version probe failed: {}",
        String::from_utf8_lossy(&report.stderr)
    );
    let version: release::Version = std::str::from_utf8(&report.stdout)?
        .trim()
        .strip_prefix("kuru ")
        .context("native fixture binary did not report kuru VERSION")?
        .parse()?;
    if let Ok(selected) = std::env::var("RELEASE_VERSION") {
        ensure!(
            selected.parse::<release::Version>()? == version,
            "RELEASE_VERSION differs from native fixture executable"
        );
    }
    let generated = root.join("generated-shell-support");
    for (name, args) in [
        ("completions/kuru.bash", &["completions", "bash"][..]),
        ("completions/_kuru", &["completions", "zsh"][..]),
        ("completions/kuru.fish", &["completions", "fish"][..]),
        ("completions/kuru.ps1", &["completions", "powershell"][..]),
        ("man/kuru.1", &["man"][..]),
    ] {
        let output =
            command::bounded_output(launch().args(args), Duration::from_secs(20), OUTPUT_LIMIT)
                .await?;
        ensure!(
            output.status.success(),
            "native fixture binary could not generate {name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let destination = generated.join(name);
        fs::create_dir_all(destination.parent().context("support file has no parent")?)?;
        fs::write(destination, output.stdout)?;
    }
    ensure!(
        !generator.join("data").exists(),
        "pure fixture generation opened application memory"
    );
    let output = root.join("simulated-release");
    eprintln!(
        "Homebrew acceptance: package simulated v{version} release from native {} executable; foreign target archives are shape fixtures, not architecture or signing acceptance",
        targets::host()?.triple
    );
    package_simulated_release(&binary, &generated, version, &output)?;
    Ok((output, version))
}

async fn acceptance(root: &Path) -> Result<()> {
    let target = targets::host()?;
    ensure!(
        target.os == "macos" || target.os == "linux",
        "Homebrew acceptance needs a supported native Unix target"
    );
    let (directory, version) =
        if let Some(binary) = std::env::var_os("KURU_HOMEBREW_CANDIDATE_BINARY") {
            ensure!(
                std::env::var_os("KURU_HOMEBREW_RELEASE_DIR").is_none(),
                "select only one Homebrew candidate input"
            );
            simulated_release(root, PathBuf::from(binary)).await?
        } else {
            let directory = PathBuf::from(std::env::var_os("KURU_HOMEBREW_RELEASE_DIR").context(
                "KURU_HOMEBREW_RELEASE_DIR or KURU_HOMEBREW_CANDIDATE_BINARY is required",
            )?);
            ensure!(
                directory.is_absolute(),
                "Homebrew release directory must be absolute"
            );
            let version: release::Version = std::env::var("RELEASE_VERSION")
                .context("RELEASE_VERSION is required for assembled candidates")?
                .parse()?;
            (directory, version)
        };
    let hashes = release::verified_assets(&directory, version)?;
    let formula = homebrew::generate(version, homebrew::SOURCE_REPOSITORY, &hashes)?;
    let name = archive::archive_name(&version.to_string(), target.triple)?;
    let core = fs::read(directory.join(&name))?;
    let (binary, marked) = archive::extract_core(&core, target, archive::MAX_ARCHIVE_BYTES)?;
    ensure!(marked, "Homebrew candidate requires paired support");
    let support_name = shell_support::archive_name(&version.to_string(), target.triple)?;
    let envelope = fs::read(directory.join(&support_name))?;
    let support = shell_support::decode(&envelope, target)?;
    let token = published::checked_token(std::env::var_os("GITHUB_TOKEN"))?;
    let predecessor =
        published::previous_release(&version.to_string(), target.triple, token.as_deref()).await?;

    fs::create_dir(root.join("empty-hooks"))?;
    let installed = text(brew(&["list", "--formula", "--full-name"], root).await?)?;
    ensure!(
        !installed
            .lines()
            .any(|name| name.rsplit('/').next() == Some("kuru")),
        "refusing to modify a pre-existing Homebrew Kuru installation"
    );
    let cellar = PathBuf::from(text(brew(&["--cellar"], root).await?)?);
    ensure!(
        !cellar.join("kuru").exists(),
        "refusing an existing Kuru Cellar directory"
    );
    let mut files = BTreeMap::from([
        (format!("/v{version}/{name}"), core.clone()),
        (format!("/v{version}/{support_name}"), envelope),
    ]);
    let old_formula = if let published::Predecessor::Release(previous) = &predecessor {
        let old_name = archive::archive_name(&previous.version, target.triple)?;
        let mut old = formula
            .replace(&version.to_string(), &previous.version)
            .replace(&hashes[&name], &archive::digest(&previous.archive));
        files.insert(
            format!("/v{}/{old_name}", previous.version),
            previous.archive.clone(),
        );
        if let Some(envelope) = &previous.support {
            let old_support = shell_support::archive_name(&previous.version, target.triple)?;
            old = old.replace(&hashes[&support_name], &archive::digest(envelope));
            files.insert(
                format!("/v{}/{old_support}", previous.version),
                envelope.clone(),
            );
        } else {
            // Historical unmarked releases had no support envelope. This
            // fixture-only formula installs the authenticated old core; the
            // upgrade must add the candidate's complete support inventory.
            let mut skipping = false;
            old = old
                .lines()
                .filter(|line| {
                    if *line == "      resource \"shell-support\" do" {
                        skipping = true;
                        return false;
                    }
                    if skipping {
                        if *line == "      end" {
                            skipping = false;
                        }
                        return false;
                    }
                    true
                })
                .collect::<Vec<_>>()
                .join("\n");
            old = old
                .split("    resource(\"shell-support\").stage do")
                .next()
                .context("old formula install boundary")?
                .to_owned()
                + "  end\nend\n";
        }
        Some(old)
    } else {
        None
    };
    let assets = Arc::new(Assets {
        files,
        requests: AtomicUsize::new(0),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}/", listener.local_addr()?);
    let app = Router::new().fallback(serve).with_state(assets.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let git = fixture_git::FixtureGit::new();
    let repository = root.join("tap-source");
    fs::create_dir_all(repository.join("Formula"))?;
    ensure!(
        git.git(&repository, &["init", "-b", "main"])
            .await
            .status
            .success(),
        "fixture Git init failed"
    );
    commit_formula(
        &git,
        &repository,
        &local_formula(old_formula.as_deref().unwrap_or(&formula), &base)?,
    )
    .await?;
    let tap = format!("kuru-acceptance-{}/test", uuid::Uuid::new_v4().simple());
    let full_name = format!("{tap}/kuru");
    brew(
        &[
            "tap",
            "--custom-remote",
            &tap,
            repository.to_str().context("tap path not UTF-8")?,
        ],
        root,
    )
    .await?;
    let result = async {
        brew(&["install", "--formula", &full_name], root).await?;
        if let published::Predecessor::Release(previous) = &predecessor {
            let keg = cellar.join("kuru").join(&previous.version);
            let (old_binary, _) = archive::extract_core(&previous.archive, target, archive::MAX_ARCHIVE_BYTES)?;
            let old_support = previous.support.as_ref().map(|envelope| shell_support::decode(envelope, target)).transpose()?;
            exact_install(&keg, &old_binary, old_support.as_ref(), &previous.archive)?;
            let mut old_runtime = Offline::new(&root.join("old-runtime"), keg.join("bin/kuru"), cellar.clone())?;
            let old_session = old_runtime.conversation().await?;
            let tap_repository = PathBuf::from(text(brew(&["--repository", &tap], root).await?)?);
            // Advance only this owned tap's formula. Plain add/commit retain
            // the fixture's childless Git audit; brew performs the real upgrade.
            commit_formula(&git, &tap_repository, &local_formula(&formula, &base)?).await?;
            brew(&["upgrade", "--formula", &full_name], root).await?;
            old_runtime.binary = cellar.join("kuru").join(version.to_string()).join("bin/kuru");
            let resumed = old_runtime.run(&["--resume", &old_session, "run", "Continue the conversation after Homebrew upgrade", "--json"]).await?;
            ensure!(resumed["session"] == old_session, "Homebrew upgrade lost the previous release's durable conversation");
            old_runtime.quiescent().await?;
            println!("Homebrew upgraded native {} v{} to v{version}", target.triple, previous.version);
        } else if let Some(evidence) = predecessor.evidence() { println!("{evidence}"); }
        let keg = cellar.join("kuru").join(version.to_string());
        exact_install(&keg, &binary, Some(&support), &core)?;
        brew(&["test", &full_name], root).await?;
        let offline = Offline::new(&root.join("candidate-runtime"), keg.join("bin/kuru"), cellar.clone())?;
        let before = assets.requests.load(Ordering::Relaxed);
        for args in [vec!["update", "--version", "0.0.0", "--release-base", &base], vec!["update", "--source", "/does-not-exist"]] {
            let output = command::bounded_output(offline.command().args(&args), Duration::from_secs(20), OUTPUT_LIMIT).await?;
            ensure!(!output.status.success() && String::from_utf8_lossy(&output.stderr).contains("brew upgrade kuru"), "Homebrew self-update failed to refuse before effects: {}", String::from_utf8_lossy(&output.stderr));
        }
        ensure!(assets.requests.load(Ordering::Relaxed) == before && !keg.join("bin/.kuru-update").exists() && !offline.root.join("data").exists(), "Homebrew self-update caused network, staging or memory effects");
        offline.conversation().await?;
        exact_install(&keg, &binary, Some(&support), &core)?;
        println!("Homebrew accepted {} v{version}: exact executable, documents, four completions and man; offline cold conversation, durable reopen and process cleanup", target.triple);
        Ok::<_, anyhow::Error>(())
    }.await;
    // Failure paths can leave a service completing cleanup after its client
    // exited. Prove quiescence before asking Homebrew to remove its executable,
    // and retain the tap, keg and private source if that proof is unavailable.
    for label in ["old-runtime", "candidate-runtime"] {
        let runtime_root = root.join(label);
        if runtime_root.exists()
            && let Err(cleanup) = wait_for_quiescence(&runtime_root).await
        {
            server.abort();
            return Err(cleanup).with_context(|| format!("Homebrew acceptance outcome: {}; retained fixture tap {tap}, installed Kuru under {} and private source/runtime at {}; no uninstall attempted", result.as_ref().err().map_or_else(|| "checks passed".to_owned(), |error| format!("{error:#}")), cellar.join("kuru").display(), root.display()));
        }
    }
    let uninstall = brew(
        &[
            "uninstall",
            "--ignore-dependencies",
            "--formula",
            &full_name,
        ],
        root,
    )
    .await;
    server.abort();
    let outcome = result
        .as_ref()
        .err()
        .map_or_else(|| "checks passed".to_owned(), |error| format!("{error:#}"));
    uninstall.with_context(|| format!("Homebrew acceptance outcome: {outcome}; retained fixture tap {tap} and private source at {} because Kuru uninstall did not finish", repository.display()))?;
    brew(&["untap", &tap], root).await.with_context(|| format!("Homebrew acceptance outcome: {outcome}; fixture tap {tap} could not be removed; private source retained at {}", repository.display()))?;
    result
}

#[test]
fn local_asset_substitution_accepts_only_literal_loopback_version_roots() {
    let formula =
        "url \"https://github.com/replygirl/kuru/releases/download/v0.12.0/kuru.tar.gz\"\n";
    assert_eq!(
        local_formula(formula, "http://127.0.0.1:12345/").unwrap(),
        "url \"http://127.0.0.1:12345/v0.12.0/kuru.tar.gz\"\n"
    );
    for base in [
        "https://127.0.0.1:12345/",
        "http://localhost:12345/",
        "http://127.0.0.1/",
        "http://example.com:12345/",
        "http://127.0.0.1:12345/other/",
        "http://user@127.0.0.1:12345/",
        "http://127.0.0.1:12345/?token=fake",
        "http://127.0.0.1:12345/#fragment",
    ] {
        assert!(local_formula(formula, base).is_err(), "accepted {base}");
    }
}

#[test]
fn simulated_release_has_exact_catalog_shapes_and_preserves_input_and_support() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("host-binary");
    let bytes = b"opaque native fixture executable bytes";
    fs::write(&input, bytes).unwrap();
    let input_file = fs::File::open(&input).unwrap();
    kuru_platform::fs::make_executable(&input_file).unwrap();
    let identity = kuru_platform::fs::regular_file_info(&input_file)
        .unwrap()
        .identity;
    let generated = root.path().join("generated");
    for (index, name) in shell_support::NAMES.into_iter().enumerate() {
        let path = generated.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, format!("paired native fixture support {index}\n")).unwrap();
    }
    let expected_support = shell_support::read_generated(&generated).unwrap();
    let output = root.path().join("simulated-release");
    let version: release::Version = "0.11.0".parse().unwrap();
    package_simulated_release(&input, &generated, version, &output).unwrap();
    assert_eq!(fs::read(&input).unwrap(), bytes);
    assert_eq!(
        kuru_platform::fs::regular_file_info(&input_file)
            .unwrap()
            .identity,
        identity
    );
    let hashes = release::verified_assets(&output, version).unwrap();
    assert_eq!(hashes.len(), 11);
    assert_eq!(fs::read_dir(&output).unwrap().count(), 21);
    let manifest = fs::read(output.join("SHA256SUMS")).unwrap();
    for target in targets::CATALOG {
        let core_name = archive::archive_name(&version.to_string(), target.triple).unwrap();
        let archive = fs::read(output.join(&core_name)).unwrap();
        assert_eq!(
            archive::expected_digest(&manifest, &core_name).unwrap(),
            archive::digest(&archive)
        );
        let (executable, marked) =
            archive::extract_core(&archive, &target, archive::MAX_ARCHIVE_BYTES).unwrap();
        assert_eq!(executable, bytes);
        assert!(marked);
        let support_name =
            shell_support::archive_name(&version.to_string(), target.triple).unwrap();
        let support_archive = fs::read(output.join(&support_name)).unwrap();
        assert_eq!(
            archive::expected_digest(&manifest, &support_name).unwrap(),
            archive::digest(&support_archive)
        );
        assert_eq!(
            shell_support::decode(&support_archive, &target).unwrap(),
            expected_support
        );
    }
}

#[tokio::test]
#[ignore = "requires native Homebrew, assembled candidate or native release binary, and previous public release access"]
async fn native_homebrew_install_and_upgrade() {
    let root = tempfile::Builder::new()
        .prefix("kuru-homebrew-acceptance-")
        .tempdir()
        .unwrap();
    if let Err(error) = acceptance(root.path()).await {
        let path = root.keep();
        panic!(
            "{error:#}\nHomebrew acceptance private diagnostics retained at {}",
            path.display()
        );
    }
    root.close().unwrap();
}
