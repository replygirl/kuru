//! Bounded RustSec advisory database maintenance and offline lockfile scans.

use anyhow::{Context, Result, anyhow, ensure};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Output,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const ORIGIN: &str = "https://github.com/RustSec/advisory-db.git";
const FETCH_REFSPEC: &str = "+refs/heads/*:refs/remotes/origin/*";
const MAXIMUM_AGE: Duration = Duration::from_secs(90 * 24 * 60 * 60);
const OUTPUT_LIMIT: usize = 4 * 1024 * 1024;

/// A validated, clean and detached RustSec advisory database revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DatabaseRevision {
    pub sha: String,
    pub committed_at: u64,
}

impl DatabaseRevision {
    fn print(&self, stage: &str) {
        println!(
            "RustSec advisory database {stage}: {} ({})",
            self.sha, self.committed_at
        );
    }
}

struct GitEnvironment {
    _temporary: tempfile::TempDir,
    global_config: PathBuf,
    hooks: PathBuf,
}

impl GitEnvironment {
    fn new() -> Result<Self> {
        let temporary = tempfile::tempdir().context("create isolated Git settings directory")?;
        let global_config = temporary.path().join("global.gitconfig");
        let hooks = temporary.path().join("hooks");
        fs::write(&global_config, b"").context("create empty Git global configuration")?;
        fs::create_dir(&hooks).context("create empty Git hooks directory")?;
        Ok(Self {
            _temporary: temporary,
            global_config,
            hooks,
        })
    }

    fn configure_environment(&self, command: &mut crate::command::Command) {
        // rooted clears Git's repository selectors. Config injection also has
        // indexed key/value variables, so remove every inherited variant before
        // setting this process's controlled configuration locations.
        for (key, _) in std::env::vars_os() {
            let key = key.to_string_lossy();
            if key == "GIT_CONFIG"
                || key == "GIT_CONFIG_PARAMETERS"
                || key == "GIT_CONFIG_COUNT"
                || key == "GIT_CONFIG_GLOBAL"
                || key == "GIT_CONFIG_SYSTEM"
                || key.starts_with("GIT_CONFIG_KEY_")
                || key.starts_with("GIT_CONFIG_VALUE_")
            {
                command.env_remove(key.as_ref());
            }
        }
        for key in [
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_CONFIG",
            "GIT_CONFIG_PARAMETERS",
            "GIT_CONFIG_COUNT",
            "GIT_CONFIG_GLOBAL",
            "GIT_CONFIG_SYSTEM",
            "GIT_OBJECT_DIRECTORY",
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_IMPLICIT_WORK_TREE",
            "GIT_GRAFT_FILE",
            "GIT_INDEX_FILE",
            "GIT_NO_REPLACE_OBJECTS",
            "GIT_REPLACE_REF_BASE",
            "GIT_PREFIX",
            "GIT_SHALLOW_FILE",
            "GIT_COMMON_DIR",
        ] {
            command.env_remove(key);
        }
        for index in 0..=64 {
            command.env_remove(format!("GIT_CONFIG_KEY_{index}"));
            command.env_remove(format!("GIT_CONFIG_VALUE_{index}"));
        }
        command
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", &self.global_config)
            .env("GIT_TERMINAL_PROMPT", "0");
    }

    fn configure_git(&self, command: &mut crate::command::Command) {
        self.configure_environment(command);
        command.args(["-c", &format!("core.hooksPath={}", self.hooks.display())]);
    }
}

fn checked_database(database: &Path) -> Result<()> {
    ensure!(
        database.is_absolute(),
        "KURU_ADVISORY_DB must be an absolute path"
    );
    ensure!(
        database.file_name().is_some(),
        "KURU_ADVISORY_DB must name an isolated checkout directory"
    );
    Ok(())
}

async fn git(
    environment: &GitEnvironment,
    directory: &Path,
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<Output> {
    let mut command = crate::command::rooted(directory, "git");
    environment.configure_git(&mut command);
    command.args(arguments);
    let output =
        crate::command::bounded_output(&mut command, Duration::from_secs(60), OUTPUT_LIMIT)
            .await
            .context("run bounded advisory Git command")?;
    Ok(output)
}

fn successful(label: &str, output: Output) -> Result<Vec<u8>> {
    ensure!(
        output.status.success(),
        "{label} failed ({})",
        output.status
    );
    Ok(output.stdout)
}

fn parse_config(bytes: &[u8]) -> Result<BTreeMap<String, String>> {
    let text = std::str::from_utf8(bytes).context("advisory Git config was not UTF-8")?;
    let mut values = BTreeMap::new();
    for record in text.split('\0').filter(|record| !record.is_empty()) {
        let (key, value) = record
            .split_once('\n')
            .ok_or_else(|| anyhow!("advisory Git config has an invalid record"))?;
        ensure!(
            values.insert(key.to_owned(), value.to_owned()).is_none(),
            "advisory Git config repeats {key}"
        );
    }
    Ok(values)
}

fn boolean(value: &str) -> bool {
    matches!(value, "true" | "false")
}

fn validate_config(values: &BTreeMap<String, String>, origin: &str) -> Result<()> {
    for (key, value) in values {
        let allowed = match key.as_str() {
            "core.repositoryformatversion" => value == "0",
            "core.filemode" | "core.ignorecase" | "core.precomposeunicode" => boolean(value),
            "core.symlinks" => value == "false",
            "core.bare" => value == "false",
            "core.logallrefupdates" => value == "true",
            "remote.origin.url" => value == origin,
            "remote.origin.fetch" => value == FETCH_REFSPEC,
            _ => false,
        };
        ensure!(
            allowed,
            "advisory database local configuration is not an accepted fresh clone"
        );
    }
    for key in [
        "core.repositoryformatversion",
        "core.filemode",
        "core.bare",
        "core.logallrefupdates",
        "remote.origin.url",
        "remote.origin.fetch",
    ] {
        ensure!(
            values.contains_key(key),
            "advisory database local configuration is missing {key}"
        );
    }
    Ok(())
}

async fn local_config(
    environment: &GitEnvironment,
    database: &Path,
    origin: &str,
) -> Result<BTreeMap<String, String>> {
    let output = git(
        environment,
        database,
        [
            OsString::from("config"),
            OsString::from("--local"),
            OsString::from("--null"),
            OsString::from("--list"),
            OsString::from("--no-includes"),
        ],
    )
    .await?;
    let values = parse_config(&successful(
        "read advisory database local configuration",
        output,
    )?)?;
    validate_config(&values, origin)?;
    Ok(values)
}

async fn inspect_with(
    environment: &GitEnvironment,
    database: &Path,
    require_fresh: bool,
    origin: &str,
) -> Result<DatabaseRevision> {
    checked_database(database)?;
    ensure!(
        database.is_dir(),
        "advisory database is missing; run audit:advisories:refresh"
    );
    ensure!(
        database.join(".git").is_dir(),
        "advisory database must be a standalone checkout root"
    );
    let database_root = database
        .canonicalize()
        .context("canonicalize advisory database root")?;
    let reported_root = String::from_utf8(successful(
        "read advisory database checkout root",
        git(
            environment,
            database,
            [
                OsString::from("rev-parse"),
                OsString::from("--show-toplevel"),
            ],
        )
        .await?,
    )?)?;
    ensure!(
        Path::new(reported_root.trim()).canonicalize()? == database_root,
        "advisory database path must be the checkout root"
    );
    local_config(environment, database, origin).await?;

    let status = successful(
        "inspect advisory database working tree",
        git(
            environment,
            database,
            [
                OsString::from("status"),
                OsString::from("--porcelain=v1"),
                OsString::from("--untracked-files=all"),
            ],
        )
        .await?,
    )?;
    ensure!(status.is_empty(), "advisory database must be clean");

    let detached = git(
        environment,
        database,
        [
            OsString::from("symbolic-ref"),
            OsString::from("-q"),
            OsString::from("HEAD"),
        ],
    )
    .await?;
    ensure!(
        detached.status.code() == Some(1),
        "advisory database HEAD must be detached"
    );

    let sha = String::from_utf8(successful(
        "read advisory database HEAD",
        git(
            environment,
            database,
            [
                OsString::from("rev-parse"),
                OsString::from("--verify"),
                OsString::from("HEAD^{commit}"),
            ],
        )
        .await?,
    )?)?
    .trim()
    .to_owned();
    ensure!(
        sha.len() == 40 && sha.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "advisory database HEAD must be a full SHA-1 commit ID"
    );

    let committed_at: u64 = String::from_utf8(successful(
        "read advisory database commit timestamp",
        git(
            environment,
            database,
            [
                OsString::from("show"),
                OsString::from("-s"),
                OsString::from("--format=%ct"),
                OsString::from("HEAD"),
            ],
        )
        .await?,
    )?)?
    .trim()
    .parse()
    .context("advisory database commit timestamp was invalid")?;
    let committed = UNIX_EPOCH
        .checked_add(Duration::from_secs(committed_at))
        .ok_or_else(|| anyhow!("advisory database commit timestamp overflowed"))?;
    let age = SystemTime::now()
        .duration_since(committed)
        .map_err(|_| anyhow!("advisory database commit timestamp is in the future"))?;
    if require_fresh {
        ensure!(
            age <= MAXIMUM_AGE,
            "advisory database is older than 90 days; run audit:advisories:refresh"
        );
    }
    Ok(DatabaseRevision { sha, committed_at })
}

/// Inspect a previously refreshed database without mutating it.
pub async fn inspect(database: &Path) -> Result<DatabaseRevision> {
    let environment = GitEnvironment::new()?;
    inspect_with(&environment, database, true, ORIGIN).await
}

/// Create or explicitly refresh an isolated public RustSec checkout.
pub async fn refresh(database: &Path) -> Result<DatabaseRevision> {
    refresh_with_origin(database, ORIGIN).await
}

async fn refresh_with_origin(database: &Path, origin: &str) -> Result<DatabaseRevision> {
    checked_database(database)?;
    let environment = GitEnvironment::new()?;
    if database.exists() {
        inspect_with(&environment, database, false, origin).await?;
        successful(
            "refresh advisory database",
            git(
                &environment,
                database,
                [
                    OsString::from("fetch"),
                    OsString::from("--no-tags"),
                    OsString::from("--prune"),
                    OsString::from("origin"),
                    OsString::from("HEAD"),
                ],
            )
            .await?,
        )?;
        successful(
            "detach refreshed advisory database HEAD",
            git(
                &environment,
                database,
                [
                    OsString::from("checkout"),
                    OsString::from("--detach"),
                    OsString::from("FETCH_HEAD"),
                ],
            )
            .await?,
        )?;
    } else {
        let parent = database
            .parent()
            .ok_or_else(|| anyhow!("KURU_ADVISORY_DB must have a parent directory"))?;
        ensure!(
            parent.is_dir(),
            "advisory database parent directory is missing"
        );
        fs::create_dir(database).context("create isolated advisory database directory")?;
        successful(
            "initialize advisory database",
            git(&environment, database, [OsString::from("init")]).await?,
        )?;
        successful(
            "configure advisory database origin",
            git(
                &environment,
                database,
                [
                    OsString::from("remote"),
                    OsString::from("add"),
                    OsString::from("origin"),
                    OsString::from(origin),
                ],
            )
            .await?,
        )?;
        successful(
            "configure advisory database fetch refspec",
            git(
                &environment,
                database,
                [
                    OsString::from("config"),
                    OsString::from("remote.origin.fetch"),
                    OsString::from(FETCH_REFSPEC),
                ],
            )
            .await?,
        )?;
        local_config(&environment, database, origin).await?;
        successful(
            "fetch advisory database origin HEAD",
            git(
                &environment,
                database,
                [
                    OsString::from("fetch"),
                    OsString::from("--no-tags"),
                    OsString::from("origin"),
                    OsString::from("HEAD"),
                ],
            )
            .await?,
        )?;
        successful(
            "detach fetched advisory database HEAD",
            git(
                &environment,
                database,
                [
                    OsString::from("checkout"),
                    OsString::from("--detach"),
                    OsString::from("FETCH_HEAD"),
                ],
            )
            .await?,
        )?;
    }
    let revision = inspect_with(&environment, database, true, origin).await?;
    revision.print("refreshed");
    Ok(revision)
}

/// Scan the root Cargo lockfile with a direct, mise-selected cargo-audit binary.
pub async fn scan(database: &Path, audit_binary: &Path) -> Result<DatabaseRevision> {
    checked_database(database)?;
    ensure!(
        audit_binary.is_absolute(),
        "--audit-bin must be an absolute mise-selected path"
    );
    ensure!(audit_binary.is_file(), "--audit-bin does not name a file");
    let package = std::env::current_dir().context("read delivery package directory")?;
    ensure!(
        package.join(".cargo/audit.toml").is_file(),
        "run audit:advisories from packages/kuru-delivery"
    );
    let lockfile = package
        .join("../..")
        .join("Cargo.lock")
        .canonicalize()
        .context("resolve root Cargo.lock from delivery package")?;
    let environment = GitEnvironment::new()?;
    let before = inspect_with(&environment, database, true, ORIGIN).await?;
    before.print("before scan");

    let mut command = crate::command::rooted(&package, audit_binary);
    environment.configure_environment(&mut command);
    // cargo-audit 0.22.2 consults the project configuration before Cargo Home,
    // but still receives an owned empty Cargo Home so an inherited user policy
    // cannot become its fallback configuration.
    let cargo_home =
        tempfile::tempdir().context("create isolated cargo-audit configuration home")?;
    command.env("CARGO_HOME", cargo_home.path());
    command.args([
        "audit",
        "--file",
        lockfile
            .to_str()
            .ok_or_else(|| anyhow!("root Cargo.lock path was not UTF-8"))?,
        "--db",
        database
            .to_str()
            .ok_or_else(|| anyhow!("advisory database path was not UTF-8"))?,
        "--no-fetch",
        "--no-yanked",
    ]);
    let output =
        crate::command::bounded_output(&mut command, Duration::from_secs(180), OUTPUT_LIMIT)
            .await
            .context("run bounded cargo-audit scan")?;
    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    let after = inspect_with(&environment, database, true, ORIGIN).await?;
    after.print("after scan");
    ensure!(
        before == after,
        "advisory database changed during offline scan"
    );
    ensure!(
        output.status.success(),
        "cargo-audit scan failed ({})",
        output.status
    );
    Ok(after)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn git(directory: &Path, arguments: &[&str]) {
        let environment = GitEnvironment::new().unwrap();
        let mut command = crate::command::rooted(directory, "git");
        // The helper begins with hostile inherited selectors/config injection;
        // the production environment must remove them before invoking Git.
        command
            .env("GIT_DIR", "foreign-directory")
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "core.hooksPath")
            .env("GIT_CONFIG_VALUE_0", "foreign-hooks");
        environment.configure_git(&mut command);
        command.args(arguments);
        let output =
            crate::command::bounded_output(&mut command, Duration::from_secs(10), OUTPUT_LIMIT)
                .await
                .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    async fn git_with_environment(
        directory: &Path,
        arguments: &[&str],
        environment: &[(&str, &str)],
    ) {
        let environment_settings = GitEnvironment::new().unwrap();
        let mut command = crate::command::rooted(directory, "git");
        environment_settings.configure_git(&mut command);
        command.args(arguments).envs(environment.iter().copied());
        let output =
            crate::command::bounded_output(&mut command, Duration::from_secs(10), OUTPUT_LIMIT)
                .await
                .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    async fn database() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("advisory-db");
        fs::create_dir(&database).unwrap();
        git(&database, &["init"]).await;
        fs::write(database.join("README.md"), b"fixture\n").unwrap();
        git(&database, &["add", "README.md"]).await;
        git(
            &database,
            &[
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-m",
                "fixture",
            ],
        )
        .await;
        git(&database, &["remote", "add", "origin", ORIGIN]).await;
        git(&database, &["config", "remote.origin.fetch", FETCH_REFSPEC]).await;
        git(&database, &["checkout", "--detach", "HEAD"]).await;
        root
    }

    #[tokio::test]
    async fn accepts_fresh_clone_bookkeeping_and_rejects_executable_local_config() {
        let root = database().await;
        let database = root.path().join("advisory-db");
        let revision = inspect(&database).await.unwrap();
        assert_eq!(revision.sha.len(), 40);

        git(&database, &["config", "core.hooksPath", "fixture-hooks"]).await;
        let error = inspect(&database).await.unwrap_err().to_string();
        assert_eq!(
            error,
            "advisory database local configuration is not an accepted fresh clone"
        );
    }

    #[tokio::test]
    async fn fresh_owned_setup_has_only_allowlisted_config_and_checked_values() {
        let root = database().await;
        let database = root.path().join("advisory-db");
        let environment = GitEnvironment::new().unwrap();
        let values = local_config(&environment, &database, ORIGIN).await.unwrap();
        assert!(!values.keys().any(|key| key.starts_with("branch.")));

        let mut values = BTreeMap::from([
            ("core.repositoryformatversion".into(), "0".into()),
            ("core.filemode".into(), "true".into()),
            ("core.bare".into(), "false".into()),
            ("core.logallrefupdates".into(), "true".into()),
            ("remote.origin.url".into(), ORIGIN.into()),
            ("remote.origin.fetch".into(), FETCH_REFSPEC.into()),
        ]);
        validate_config(&values, ORIGIN).unwrap();
        values.insert("core.symlinks".into(), "false".into());
        validate_config(&values, ORIGIN).unwrap();
        values.insert("core.symlinks".into(), "true".into());
        assert!(validate_config(&values, ORIGIN).is_err());
        values.insert("core.symlinks".into(), "false".into());
        values.insert("core.bare".into(), "true".into());
        assert!(validate_config(&values, ORIGIN).is_err());
        values.insert("core.bare".into(), "false".into());
        values.insert(
            "remote.origin.url".into(),
            "https://example.invalid/other.git".into(),
        );
        assert!(validate_config(&values, ORIGIN).is_err());
    }

    #[tokio::test]
    async fn refresh_accepts_stale_expected_checkout_before_fetching_a_local_test_origin() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let database = root.path().join("database");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&database).unwrap();
        git(&source, &["init"]).await;
        fs::write(source.join("README.md"), b"fresh source\n").unwrap();
        git(&source, &["add", "README.md"]).await;
        git(
            &source,
            &[
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "fresh source",
            ],
        )
        .await;
        let origin = source.to_str().unwrap();
        git(&database, &["init"]).await;
        git(&database, &["remote", "add", "origin", origin]).await;
        git(&database, &["config", "remote.origin.fetch", FETCH_REFSPEC]).await;
        git(&database, &["fetch", "--no-tags", "origin", "HEAD"]).await;
        git(&database, &["checkout", "--detach", "FETCH_HEAD"]).await;
        git_with_environment(
            &database,
            &[
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--allow-empty",
                "-m",
                "stale local checkout",
                "--date=2000-01-01T00:00:00Z",
            ],
            &[
                ("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z"),
                ("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z"),
            ],
        )
        .await;

        let refreshed = refresh_with_origin(&database, origin).await.unwrap();
        assert!(refreshed.committed_at > 946_684_800);
    }
}
