use std::path::{Path, PathBuf};

use anyhow::Context as _;

/// Retires managed memory owners created by one application fixture before its
/// temporary data root is removed. A service retires itself as soon as its last
/// client detaches but may still be closing when that client exits, so
/// subprocess fixtures must own this final step and await the close rather than
/// leave independent Dolt processes accumulating across the parallel
/// application suite.
///
/// It also owns the fixture's owner diagnostic file: a command-line child
/// given [`ServiceCleanup::owner_diagnostic_path`] through
/// `kuru_memory::test_support::OWNER_DIAGNOSTIC_ENV` sends the stderr of any
/// owner it elects there. Dropping the fixture writes that stderr to the
/// test's captured output, and [`ServiceCleanup::release`] attaches it to a
/// failed outcome, before the root is removed.
pub struct ServiceCleanup {
    data: Vec<PathBuf>,
    root: Option<kuru_memory::test_support::TempDir>,
    owner_diagnostic: PathBuf,
    pending: bool,
}

/// The name, in each fixture's private root, of its owner diagnostic file.
const OWNER_DIAGNOSTIC: &str = "owner-diagnostic.log";

/// How the memory server names a failed Dolt start's private log
/// (`packages/kuru-memory/src/server.rs`), which owner stderr carries verbatim
/// in its `{error:#}` chain: this prefix, the store directory joined with
/// [`SERVER_LOG`], then `": "` and Dolt's own failure.
const PRIVATE_DIAGNOSTICS: &str = "Dolt startup/lifetime failed; private diagnostics: ";
/// The file name that producer joins to the store directory.
const SERVER_LOG: &str = "server.log";

/// The bounded tail of each Dolt log `owner_text` names after
/// [`PRIVATE_DIAGNOSTICS`], one line per distinct path; empty when it names
/// none. A read tail carries the helper's own label, which names the path; a
/// path that is not read is named once before the reason.
///
/// Owner stderr is untrusted text. A named path is taken through the first
/// [`SERVER_LOG`] that ends the line or is followed by `':'`, so a Windows
/// drive colon cannot end it early. It is read only when it is absolute and
/// spelled through prefix, root and ordinary components only (no `..` or
/// `.`), and its canonical form is strictly beneath the canonical `root`.
/// Containment is decided canonically because the owner names its stage from
/// the canonical data directory (`fs::canonicalize` in
/// `packages/kuru-memory/src/server.rs`) while `root` is spelled as the
/// temporary directory gave it: macOS's `/var` alias and Windows' verbatim
/// `\\?\` prefix make the two spellings differ. The canonical comparison also
/// rejects a symlinked escape. The read is
/// [`kuru_memory::test_support::fixture_server_log`]: the checked private
/// read, which refuses symlinked components, capped at that helper's
/// `STARTUP_LOG_BYTES` (above the server's own `LOG_LIMIT` on this log) and
/// cut to its `STARTUP_TAIL_BYTES` tail, the bound kuru-memory's fixtures
/// already apply to this same log. A rejected, missing or refused path is
/// reported instead; nothing here panics.
fn named_private_diagnostics(root: &Path, owner_text: &str) -> String {
    let mut named: Vec<&str> = Vec::new();
    for line in owner_text.lines() {
        for (start, _) in line.match_indices(PRIVATE_DIAGNOSTICS) {
            let rest = &line[start + PRIVATE_DIAGNOSTICS.len()..];
            let end = rest.match_indices(SERVER_LOG).find_map(|(at, _)| {
                let end = at + SERVER_LOG.len();
                let after = &rest[end..];
                (after.is_empty() || after.starts_with(':')).then_some(end)
            });
            if let Some(path) = end.map(|end| &rest[..end])
                && !named.contains(&path)
            {
                named.push(path);
            }
        }
    }
    let mut report = String::new();
    for path in named {
        match named_log_tail(root, Path::new(path)) {
            Ok(tail) => report.push_str(&format!("\nnamed private diagnostics: {tail}")),
            Err(reason) => {
                report.push_str(&format!("\nnamed private diagnostics ({path}): {reason}"));
            }
        }
    }
    report
}

/// The helper's tail of `log`, labelled with that path, or why it was not
/// read.
fn named_log_tail(root: &Path, log: &Path) -> Result<String, String> {
    use std::path::Component;
    let ordinary = log.is_absolute()
        && log.components().all(|part| {
            matches!(
                part,
                Component::Prefix(_) | Component::RootDir | Component::Normal(_)
            )
        });
    if !ordinary {
        return Err("not read: not an absolute path through ordinary components".to_owned());
    }
    let canonical_root = std::fs::canonicalize(root)
        .map_err(|error| format!("not read: the fixture root does not resolve: {error}"))?;
    let canonical = std::fs::canonicalize(log)
        .map_err(|error| format!("not read: missing or unresolvable: {error}"))?;
    if canonical == canonical_root || !canonical.starts_with(&canonical_root) {
        return Err(format!(
            "not read: resolves outside the fixture root {} to {}",
            canonical_root.display(),
            canonical.display()
        ));
    }
    kuru_memory::test_support::fixture_server_log(log.to_path_buf()).ok_or_else(|| {
        "not read: the checked private read refused it (not a private single-link \
         regular file within the helper's read cap, or removed meanwhile)"
            .to_owned()
    })
}

impl ServiceCleanup {
    /// `root` is a guarded fixture root: after this cleanup awaits every
    /// project store's quiescence, its teardown checks the recorded result.
    pub fn new(root: kuru_memory::test_support::TempDir, data: &Path) -> Self {
        // The owner diagnostic hook opens an existing file for append only.
        drop(
            kuru_platform::fs::Directory::open(
                root.path(),
                kuru_platform::fs::Privacy::OwnerOnly,
                kuru_platform::fs::NameRetention::Pinned,
            )
            .and_then(|directory| directory.create_new(std::ffi::OsStr::new(OWNER_DIAGNOSTIC)))
            .expect("create the fixture's owner diagnostic file"),
        );
        Self {
            data: vec![data.to_owned()],
            owner_diagnostic: root.path().join(OWNER_DIAGNOSTIC),
            root: Some(root),
            pending: true,
        }
    }

    /// The file a command-line child names in
    /// `kuru_memory::test_support::OWNER_DIAGNOSTIC_ENV`, so the stderr of
    /// any owner it elects reaches this fixture's failure output.
    #[allow(
        dead_code,
        reason = "each integration-test crate compiles this shared support module independently"
    )]
    pub fn owner_diagnostic_path(&self) -> &Path {
        &self.owner_diagnostic
    }

    /// What every owner this fixture's children elected wrote to stderr, as
    /// a failure-message suffix; empty when no owner wrote anything. Each
    /// private Dolt log that stderr names inside this fixture's root follows
    /// as a bounded tail ([`named_private_diagnostics`]).
    pub fn owner_diagnostic(&self) -> String {
        match std::fs::read(&self.owner_diagnostic) {
            Ok(bytes) if bytes.is_empty() => String::new(),
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                // The diagnostic file sits directly in the fixture root. Take
                // the root from it rather than from `self.root`, which a failed
                // cleanup has already taken while keeping the root on disk:
                // that failure is when this report matters most.
                let named = self
                    .owner_diagnostic
                    .parent()
                    .map(|root| named_private_diagnostics(root, &text))
                    .unwrap_or_default();
                format!(
                    "\nowner stderr ({}):\n{text}{named}",
                    self.owner_diagnostic.display()
                )
            }
            Err(error) => format!(
                "\nowner stderr unreadable ({}): {error}",
                self.owner_diagnostic.display()
            ),
        }
    }

    #[allow(
        dead_code,
        reason = "each integration-test crate compiles this shared support module independently"
    )]
    pub fn path(&self) -> &Path {
        self.root.as_ref().expect("fixture root retained").path()
    }

    #[allow(
        dead_code,
        reason = "only fixtures with more than one managed data root use this extension"
    )]
    pub fn add_data(&mut self, data: &Path) {
        assert!(self.pending, "cannot extend completed fixture cleanup");
        self.data.push(data.to_owned());
    }

    pub fn finish(&mut self) -> anyhow::Result<()> {
        if !std::mem::replace(&mut self.pending, false) {
            return Ok(());
        }
        if let Err(error) = self.retire() {
            let retained = self
                .root
                .take()
                .expect("fixture root retained before cleanup")
                .keep();
            return Err(error.context(format!(
                "managed-memory cleanup failed; fixture root retained in place at {retained:?}"
            )));
        }
        Ok(())
    }

    /// Finish cleanup after the fixture's own `outcome` and release the
    /// guarded root with it. A cleanup failure or the guard's verdict is
    /// attached to the fixture's error, or returned for a successful one; it
    /// never replaces that error with a drop panic.
    #[allow(
        dead_code,
        reason = "each integration-test crate compiles this shared support module independently"
    )]
    pub fn release<T>(mut self, outcome: anyhow::Result<T>) -> anyhow::Result<T> {
        // Read after cleanup has awaited the owners it can find, so more of
        // their stderr has landed; the file outlives cleanup either way.
        let cleanup = self.finish();
        let owner_diagnostic = self.owner_diagnostic();
        let outcome = match outcome {
            Err(error) if !owner_diagnostic.is_empty() => Err(error.context(owner_diagnostic)),
            outcome => outcome,
        };
        let outcome = match (outcome, cleanup) {
            (outcome, Ok(())) => outcome,
            (Ok(_), Err(cleanup)) => Err(cleanup),
            (Err(error), Err(cleanup)) => Err(error.context(format!(
                "the fixture failed, and its managed-memory cleanup then failed: {cleanup:#}"
            ))),
        };
        match self.root.take() {
            Some(root) => root.release(outcome),
            None => outcome,
        }
    }

    /// Retire every project a managed service may have served beneath each
    /// data root, including one whose store exists only under a staging name
    /// because an owner in another process had not activated it yet.
    ///
    /// The join needs no backstop of its own: each retirement bounds its wait
    /// on a closing owner's lock release by that owner's close budget, on a
    /// thread of its own that is abandoned at the budget, and each store
    /// quiescence wait by the supervisor's reap allowance.
    fn retire(&self) -> anyhow::Result<()> {
        let mut projects = Vec::new();
        for data in &self.data {
            for scope in kuru_memory::test_support::managed_store_scopes(data)? {
                projects.push((data.clone(), scope));
            }
        }
        if projects.is_empty() {
            return Ok(());
        }
        projects.sort();
        std::thread::Builder::new()
            .name("kuru-fixture-memory-cleanup".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .context("create managed-memory cleanup runtime")?;
                runtime.block_on(async move {
                    for (data, scope) in projects {
                        let options = kuru_memory::OpenOptions::new(data, scope);
                        kuru_memory::test_support::await_managed_quiescence(&options)
                            .await
                            .with_context(|| {
                                format!("retire fixture memory owner for {}", options.project_scope)
                            })?;
                    }
                    Ok::<(), anyhow::Error>(())
                })
            })?
            .join()
            .map_err(|_| anyhow::anyhow!("managed-memory fixture cleanup thread panicked"))?
    }
}

impl Drop for ServiceCleanup {
    fn drop(&mut self) {
        // Show the owners' stderr after cleanup has awaited the owners it can
        // find and while the file still exists: the guarded root is removed
        // only after this body, or retained when cleanup fails. The test
        // harness captures this and prints it only for a failing test, which
        // covers a test that returns its error as well as one that panics.
        let panicking = std::thread::panicking();
        let pending = self.pending;
        let finished = self.finish();
        if pending {
            let owner_diagnostic = self.owner_diagnostic();
            if !owner_diagnostic.is_empty() {
                eprintln!("managed-memory fixture{owner_diagnostic}");
            }
        }
        if let Err(error) = finished {
            if panicking {
                eprintln!("managed-memory fixture cleanup failed: {error:#}");
            } else {
                panic!("managed-memory fixture cleanup failed: {error:#}");
            }
        }
    }
}

/// Block until the managed owner for `options` has released its owner lock,
/// which it does only after retiring its endpoint and reaping Dolt, and return
/// how long that took. Call it after the last client of that owner has
/// exited, while nothing else is electing. Returns at once when no owner runs.
///
/// The wait runs on its own thread and runtime so the backstop can fail the
/// test: the lock wait itself cannot be cancelled, and that thread ends with
/// the process.
#[allow(
    dead_code,
    reason = "each integration-test crate compiles this shared support module independently"
)]
pub fn await_owner_exit(options: &kuru_memory::OpenOptions) -> anyhow::Result<std::time::Duration> {
    // The owner's own close bounds sum to about 30 s (plus a query-timeout
    // write drain), so this is well beyond a healthy close: hitting it means
    // the owner's bounds themselves failed to fire.
    const OWNER_EXIT_BACKSTOP: std::time::Duration = std::time::Duration::from_secs(120);
    let options = options.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    let started = std::time::Instant::now();
    std::thread::Builder::new()
        .name("kuru-fixture-owner-exit".into())
        .spawn(move || {
            let outcome = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .context("create owner-exit runtime")
                .and_then(|runtime| {
                    runtime.block_on(kuru_memory::test_support::await_owner_release(&options))
                });
            let _ = sender.send(outcome);
        })?;
    receiver.recv_timeout(OWNER_EXIT_BACKSTOP).map_err(|_| {
        anyhow::anyhow!("the memory owner did not exit within {OWNER_EXIT_BACKSTOP:?}")
    })??;
    Ok(started.elapsed())
}

/// CLI fixtures use ordinary user configuration and the package's verified
/// offline cache; no production-only authority or ambient user store is used.
#[allow(
    dead_code,
    reason = "each integration-test crate compiles this shared support module independently"
)]
pub fn configuration(root: &Path) -> anyhow::Result<PathBuf> {
    configuration_with(root, &kuru_memory::test_support::cache_dir()?)
}

/// [`configuration`] from inside a Tokio runtime, which cannot warm the
/// shared cache synchronously.
#[allow(
    dead_code,
    reason = "each integration-test crate compiles this shared support module independently"
)]
pub async fn configuration_warmed(root: &Path) -> anyhow::Result<PathBuf> {
    configuration_with(root, &kuru_memory::test_support::warmed_cache_dir().await?)
}

/// [`configuration`] with the shared cache a caller already warmed: from
/// inside a Tokio runtime, `kuru_memory::test_support::warmed_cache_dir()`.
#[allow(
    dead_code,
    reason = "each integration-test crate compiles this shared support module independently"
)]
pub fn configuration_with(root: &Path, cache: &Path) -> anyhow::Result<PathBuf> {
    let directory = root.join("config");
    std::fs::create_dir_all(directory.join("kuru"))?;
    let memory = kuru_core::MemoryConfig {
        cache_dir: Some(cache.to_owned()),
        offline: true,
        ..Default::default()
    };
    let config = std::collections::BTreeMap::from([("memory", memory)]);
    std::fs::write(
        directory.join("kuru/config.toml"),
        toml::to_string(&config)?,
    )?;
    Ok(directory)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuru_platform::fs::{Directory, NameRetention, Privacy};
    use std::{ffi::OsStr, io::Write as _};

    /// A private `stage/server.log` holding `bytes` beneath `root`, created
    /// through the platform's private primitives so the checked read accepts
    /// it on every OS. The path is spelled from `root` as the temporary
    /// directory gave it, which keeps macOS's `/var` alias; an owner names
    /// its stage from the canonical data directory instead, the spelling
    /// [`canonical_log`] gives.
    fn private_log(root: &Path, bytes: &[u8]) -> anyhow::Result<PathBuf> {
        let stage = Directory::open(root, Privacy::OwnerOnly, NameRetention::Movable)?
            .create_private_directory(OsStr::new("stage"))?;
        stage.create_new(OsStr::new(SERVER_LOG))?.write_all(bytes)?;
        Ok(root.join("stage").join(SERVER_LOG))
    }

    /// The same `stage/server.log` as the owner names it: `request.directory`
    /// joined with [`SERVER_LOG`], where the server has canonicalized the
    /// directory (`packages/kuru-memory/src/server.rs`). That differs from
    /// `root`'s spelling on macOS (`/private/var`) and Windows (`\\?\`) and
    /// may coincide with it on Linux.
    fn canonical_log(root: &Path) -> anyhow::Result<PathBuf> {
        Ok(std::fs::canonicalize(root)?.join("stage").join(SERVER_LOG))
    }

    /// The owner stderr line from PR #209's failed fixture, naming `log`.
    fn owner_text(log: &Path) -> String {
        format!(
            "memory service owner open failed: open and adopt the copied template stage: \
             memory server startup failed: {PRIVATE_DIAGNOSTICS}{}: Dolt exited before \
             readiness (exit status: 1)\n",
            log.display()
        )
    }

    #[test]
    fn an_in_root_log_tail_is_appended_with_its_label() -> anyhow::Result<()> {
        let root = kuru_memory::test_support::tempdir()?;
        let log = private_log(root.path(), b"dolt: fixture reason for exiting")?;
        let report = named_private_diagnostics(root.path(), &owner_text(&log));
        assert_eq!(
            report,
            format!(
                "\nnamed private diagnostics: fixture Dolt server log tail ({}): \
                 dolt: fixture reason for exiting",
                log.display()
            )
        );
        // A second mention of the same log appends it once.
        let twice = format!("{0}{0}", owner_text(&log));
        assert_eq!(named_private_diagnostics(root.path(), &twice), report);
        root.release(Ok(()))
    }

    #[test]
    fn the_owners_canonical_spelling_of_an_in_root_log_is_appended() -> anyhow::Result<()> {
        let root = kuru_memory::test_support::tempdir()?;
        private_log(root.path(), b"dolt: fixture reason for exiting")?;
        let named = canonical_log(root.path())?;
        let report = named_private_diagnostics(root.path(), &owner_text(&named));
        assert_eq!(
            report,
            format!(
                "\nnamed private diagnostics: fixture Dolt server log tail ({}): \
                 dolt: fixture reason for exiting",
                named.display()
            )
        );
        root.release(Ok(()))
    }

    #[test]
    fn only_the_bounded_tail_of_a_large_log_is_appended() -> anyhow::Result<()> {
        let root = kuru_memory::test_support::tempdir()?;
        // Twice the helper's 4 KiB tail, within its 40 KiB read cap.
        let mut bytes = b"BEGIN".to_vec();
        bytes.extend(std::iter::repeat_n(b'x', 8 * 1024));
        bytes.extend_from_slice(b"END");
        let log = private_log(root.path(), &bytes)?;
        let report = named_private_diagnostics(root.path(), &owner_text(&log));
        assert!(report.ends_with("xEND"), "{report}");
        assert!(!report.contains("BEGIN"), "{report}");
        let label = format!(
            "\nnamed private diagnostics: fixture Dolt server log tail ({}): ",
            log.display()
        );
        let tail = report
            .strip_prefix(&label)
            .context("the tail carries its label")?;
        assert!(
            tail.len() * 2 <= bytes.len(),
            "tail of {} bytes exceeds half the {}-byte log",
            tail.len(),
            bytes.len()
        );
        root.release(Ok(()))
    }

    #[test]
    fn a_log_outside_the_root_is_reported_and_not_read() -> anyhow::Result<()> {
        let root = kuru_memory::test_support::tempdir()?;
        let elsewhere = kuru_memory::test_support::tempdir()?;
        let outside = private_log(elsewhere.path(), b"OUTSIDE-SENTINEL")?;
        let absolute = named_private_diagnostics(root.path(), &owner_text(&outside));
        assert!(!absolute.contains("OUTSIDE-SENTINEL"), "{absolute}");
        assert!(
            absolute.contains(&format!(
                "({}): not read: resolves outside the fixture root",
                outside.display()
            )),
            "{absolute}"
        );

        // Each fixture root is `<temp>/<container>/private`, so
        // `<root>/../../<other container>/private/stage/server.log` names
        // the same file through the root's own prefix.
        let temp = root
            .path()
            .parent()
            .and_then(Path::parent)
            .context("fixture root container")?;
        let escape = root
            .path()
            .join("..")
            .join("..")
            .join(elsewhere.path().strip_prefix(temp)?)
            .join("stage")
            .join(SERVER_LOG);
        assert!(std::fs::metadata(&escape)?.is_file(), "escape resolves");
        let dotted = named_private_diagnostics(root.path(), &owner_text(&escape));
        assert!(!dotted.contains("OUTSIDE-SENTINEL"), "{dotted}");
        assert!(
            dotted.contains(&format!(
                "({}): not read: not an absolute path through ordinary components",
                escape.display()
            )),
            "{dotted}"
        );

        // A Windows path in the verbatim spelling the owner's canonical
        // directory has there keeps its drive colon and reaches the report
        // whole. Why it is not read depends on the host (a relative name on
        // Unix, a missing file on Windows), so only the refusal is asserted.
        let verbatim = r"\\?\C:\fixture\stage\server.log";
        let windows = named_private_diagnostics(
            root.path(),
            &format!("{PRIVATE_DIAGNOSTICS}{verbatim}: Dolt exited\n"),
        );
        assert!(
            windows.starts_with(&format!(
                "\nnamed private diagnostics ({verbatim}): not read: "
            )),
            "{windows}"
        );
        elsewhere.release(Ok(()))?;
        root.release(Ok(()))
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_escape_is_reported_and_not_read() -> anyhow::Result<()> {
        let root = kuru_memory::test_support::tempdir()?;
        let elsewhere = kuru_memory::test_support::tempdir()?;
        let outside = private_log(elsewhere.path(), b"OUTSIDE-SENTINEL")?;
        let link = root.path().join("link");
        std::os::unix::fs::symlink(outside.parent().context("stage")?, &link)?;
        let report = named_private_diagnostics(root.path(), &owner_text(&link.join(SERVER_LOG)));
        std::fs::remove_file(&link)?;
        assert!(!report.contains("OUTSIDE-SENTINEL"), "{report}");
        assert!(
            report.contains("not read: resolves outside the fixture root"),
            "{report}"
        );
        elsewhere.release(Ok(()))?;
        root.release(Ok(()))
    }

    #[test]
    fn a_missing_in_root_log_is_reported() -> anyhow::Result<()> {
        let root = kuru_memory::test_support::tempdir()?;
        let missing = root.path().join("stage").join(SERVER_LOG);
        let report = named_private_diagnostics(root.path(), &owner_text(&missing));
        assert!(
            report.starts_with(&format!(
                "\nnamed private diagnostics ({}): not read: missing or unresolvable: ",
                missing.display()
            )),
            "{report}"
        );
        assert!(named_private_diagnostics(root.path(), "no named diagnostics\n").is_empty());
        root.release(Ok(()))
    }

    #[test]
    fn the_owner_report_appends_the_named_log_after_owner_stderr() -> anyhow::Result<()> {
        let root = kuru_memory::test_support::tempdir()?;
        let data = root.path().join("data");
        private_log(root.path(), b"dolt: fixture reason for exiting")?;
        // The owner's spelling, while the fixture holds the root as given.
        let log = canonical_log(root.path())?;
        let cleanup = ServiceCleanup::new(root, &data);
        let text = owner_text(&log);
        std::fs::write(cleanup.owner_diagnostic_path(), &text)?;
        let report = cleanup.owner_diagnostic();
        // The fixture's failure output, for a reader of this test's log.
        println!("managed-memory fixture{report}");
        assert_eq!(
            report,
            format!(
                "\nowner stderr ({}):\n{text}\nnamed private diagnostics: fixture Dolt server \
                 log tail ({}): dolt: fixture reason for exiting",
                cleanup.owner_diagnostic_path().display(),
                log.display()
            )
        );
        cleanup.release(Ok(()))
    }
}
