//! Ordinary opens that create their store from the store template (cospec
//! change `memory-template-creation`): the path selector, the creation
//! worker's engine starts, its cold fallbacks, the verdicts its stage engine
//! reaches, and the fixture class guard on the shared template.
//!
//! Engine starts are counted per directory through the engine ledger
//! (`Ledger::starts_under`): a stage and the active store it becomes share a
//! key, and a build store under a private template root is counted there.
//! Tests that need an empty, busy or damaged template pass a private root
//! (`OpenOptions::template_root`), so the shared, warmed template other tests
//! copy is never disturbed.
use super::hooks::{Event, HOOKS, Hooks, LockStep, Pause, ReadFault};
use super::tests::{
    CHILD_OUTCOME, DEPTH, clone_shared, engine, ensure, entries_of, failure, hold, key,
    largest_file, published, read_manifest, rejected, release, spawn_child, wait_child,
    write_manifest,
};
use super::*;
use crate::server::{TemplateVerdict, read_identity_view};
use crate::test_support::{FreshOpen, TempDir, engine_ledger};

fn fixture() -> Result<TempDir> {
    Ok(TempDir::new("kuru-template-open-", None)?.with_depth_budget(DEPTH))
}

fn scope(digit: char) -> String {
    format!("project/{}", digit.to_string().repeat(64))
}

/// Warmed fixture options for `scope` in `data`, creating from the private
/// template root `root` when given and from the shared one otherwise.
async fn options(data: PathBuf, scope: &str, root: Option<&Path>) -> Result<OpenOptions> {
    let mut options = crate::test_support::warmed_open_options(data, scope.to_owned()).await?;
    options.template_root = root.map(Path::to_owned);
    Ok(options)
}

async fn open(options: OpenOptions) -> Result<MemoryStore> {
    crate::test_support::spawn_gated_open(options).await
}

/// Engine starts this process made on store directories first started
/// beneath `root`.
fn starts_under(root: &Path) -> Result<u64> {
    let canonical = fs::canonicalize(root)?;
    Ok(engine_ledger::with(|ledger| {
        ledger.starts_under(&canonical)
    }))
}

fn active(options: &OpenOptions) -> Result<PathBuf> {
    project_directory(&options.data_dir, &options.project_scope)
}

/// The template key the active store's identity record names.
fn template_of(options: &OpenOptions) -> Result<Option<String>> {
    Ok(read_identity_view(&active(options)?)?.template)
}

/// The entries the open preserved under its project's `interrupted/`.
fn interrupted(options: &OpenOptions) -> Result<Vec<PathBuf>> {
    let directory = active(options)?
        .parent()
        .context("store has no parent")?
        .join("interrupted");
    let mut entries = match fs::read_dir(&directory) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        entries => entries?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()?,
    };
    entries.sort();
    Ok(entries)
}

/// The project's staging directories left beside its active path.
fn stages(options: &OpenOptions) -> Result<Vec<PathBuf>> {
    let active = active(options)?;
    let prefix = format!(
        "{}.staging-",
        active
            .file_name()
            .context("store has no name")?
            .to_string_lossy()
    );
    let parent = active.parent().context("store has no parent")?;
    let mut found = Vec::new();
    match fs::read_dir(parent) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        entries => {
            for entry in entries? {
                let entry = entry?;
                if entry.file_name().to_string_lossy().starts_with(&prefix) {
                    found.push(entry.path());
                }
            }
        }
    }
    found.sort();
    Ok(found)
}

async fn count(store: &MemoryStore, query: &'static str) -> Result<u64> {
    let count: i64 = tokio::time::timeout(
        QUERY_TIMEOUT,
        sqlx::query_scalar(query).fetch_one(store.pool.as_ref()),
    )
    .await
    .with_context(|| format!("{query}: deadline exceeded"))??;
    Ok(u64::try_from(count)?)
}

/// The history lengths of `main` and the usage branch.
async fn histories(store: &MemoryStore) -> Result<(u64, u64)> {
    Ok((
        count(store, "SELECT COUNT(*) FROM dolt_log").await?,
        count(store, "SELECT COUNT(*) FROM `kuru/kuru_usage_v1`.dolt_log").await?,
    ))
}

/// The commits of a template-born store's histories before its adoption
/// commits: each carries the placeholder identity.
async fn pre_adoption_commits(store: &MemoryStore) -> Result<BTreeSet<String>> {
    let mut commits = BTreeSet::new();
    for log in ["dolt_log", "`kuru/kuru_usage_v1`.dolt_log"] {
        let found: Vec<String> = tokio::time::timeout(
            QUERY_TIMEOUT,
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT commit_hash FROM {log} WHERE commit_order < \
                 (SELECT MIN(commit_order) FROM {log} WHERE message = ?) LIMIT 1000"
            )))
            .bind(crate::server::ADOPTION_MESSAGE)
            .fetch_all(store.pool.as_ref()),
        )
        .await
        .context("pre-adoption history deadline exceeded")??;
        ensure!(!found.is_empty(), "{log} holds no pre-adoption commit");
        commits.extend(found);
    }
    Ok(commits)
}

/// The pools this store's server opened on a template-era revision: a
/// retained migration branch or a pre-adoption commit. Their identity row is
/// the placeholder, which no pool may serve.
fn template_era_pools(store: &MemoryStore, pre_adoption: &BTreeSet<String>) -> Vec<String> {
    store
        .shared
        .server
        .pool_requests()
        .into_iter()
        .filter(|name| {
            name.starts_with("kuru_migration_")
                || name.starts_with("kuru_usage_migration_")
                || pre_adoption.contains(name)
        })
        .collect()
}

/// A store this open created from the template: its identity names the
/// compiled key, and its histories are the template's plus one adoption
/// commit per ref, so no migration ran for it.
async fn assert_template_born(options: &OpenOptions, store: &MemoryStore) -> Result<()> {
    let identity = read_identity_view(&active(options)?)?;
    ensure!(
        identity.initialized && identity.template.as_deref() == Some(key()),
        "the store was not created from the template: {identity:?}"
    );
    let adopted = migrations::template_shape::compiled_commits(true)?;
    let found = histories(store).await?;
    ensure!(
        found == adopted,
        "the store's histories {found:?} are not the template's plus one adoption commit per ref \
         {adopted:?}"
    );
    Ok(())
}

/// A store the cold staged build created: no template key, and the chain's
/// own histories.
async fn assert_cold(options: &OpenOptions, store: &MemoryStore) -> Result<()> {
    ensure!(
        template_of(options)?.is_none(),
        "the store was created from a template"
    );
    let cold = migrations::template_shape::compiled_commits(false)?;
    let found = histories(store).await?;
    ensure!(
        found == cold,
        "the cold store's histories {found:?} are not {cold:?}"
    );
    Ok(())
}

/// T1: a new project on a machine with a warm engine and template opens with
/// two engine starts (the stage's adoption start and the active start), runs
/// no migration, reports `Ready` once, and builds or quarantines nothing in
/// the shared template, which the fixture guard checks at release.
#[tokio::test]
async fn warm_template_new_project_uses_two_engine_starts_and_no_migration() -> Result<()> {
    let fixture = fixture()?;
    let outcome = async {
        let shared = crate::test_support::shared_template_root()?;
        let options = options(fixture.path().join("project"), &scope('1'), None).await?;
        let template = published(&shared).context("the warmed template is not published")?;
        let (mut progress, opening) = MemoryStore::open_observed(options.clone());
        let store = {
            let _gate = crate::spawn_gate::spawning().await;
            opening.await?
        };
        let mut ready = 0;
        while let Some(stage) = progress.recv().await {
            ready += usize::from(stage == crate::MemoryOpenStage::Ready);
        }
        let checked = async {
            ensure!(ready == 1, "Ready was reported {ready} times");
            let starts = starts_under(fixture.path())?;
            ensure!(
                starts == u64::from(FreshOpen::Template.starts()),
                "a new project with a warm template made {starts} engine starts"
            );
            assert_template_born(&options, &store).await?;
            let pooled = template_era_pools(&store, &pre_adoption_commits(&store).await?);
            ensure!(pooled.is_empty(), "the open pooled {pooled:?}");
            ensure!(
                published(&shared) == Some(template),
                "the shared template changed"
            );
            ensure!(
                interrupted(&options)?.is_empty(),
                "the open preserved a stage"
            );
            Ok(())
        }
        .await;
        store.close().await?;
        checked
    }
    .await;
    fixture.release(outcome)
}

/// T2: the first project for a template key builds the template on one
/// engine, publishes it and copies itself from it: three engine starts, the
/// chain once, and nothing transient left in the template root.
#[tokio::test]
async fn first_project_builds_template_once_and_copies_with_three_starts() -> Result<()> {
    let fixture = fixture()?;
    let outcome = async {
        let root = fixture.path().join("templates");
        let options = options(fixture.path().join("first"), &scope('2'), Some(&root)).await?;
        let store = open(options.clone()).await?;
        let checked = async {
            let starts = starts_under(fixture.path())?;
            ensure!(
                starts == u64::from(FreshOpen::FirstProject.starts()),
                "the first project made {starts} engine starts"
            );
            ensure!(
                starts_under(&root)? == 1,
                "the template was not built on exactly one engine"
            );
            assert_template_born(&options, &store).await?;
            let mut names = entries_of(&root)?;
            names.sort();
            ensure!(
                names == [key().to_owned(), format!("{}.lock", key())],
                "the template root holds more than the published template: {names:?}"
            );
            Ok(())
        }
        .await;
        store.close().await?;
        checked
    }
    .await;
    fixture.release(outcome)
}

/// T3: once the first project published the template, a second project
/// copies it with two engine starts, builds nothing and leaves the template
/// and its root as they were.
#[tokio::test]
async fn second_project_reuses_template_without_build() -> Result<()> {
    let fixture = fixture()?;
    let outcome = async {
        let root = fixture.path().join("templates");
        let first = options(fixture.path().join("first"), &scope('3'), Some(&root)).await?;
        open(first).await?.close().await?;
        let template = published(&root).context("the first project published nothing")?;
        let names = entries_of(&root)?;
        let second = options(fixture.path().join("second"), &scope('4'), Some(&root)).await?;
        let store = open(second.clone()).await?;
        let checked = async {
            let starts = starts_under(&second.data_dir)?;
            ensure!(
                starts == u64::from(FreshOpen::Template.starts()),
                "the second project made {starts} engine starts"
            );
            ensure!(
                starts_under(&root)? == 1,
                "the second project started a template build"
            );
            assert_template_born(&second, &store).await?;
            ensure!(
                published(&root) == Some(template) && entries_of(&root)? == names,
                "the second project changed the template root"
            );
            Ok(())
        }
        .await;
        store.close().await?;
        checked
    }
    .await;
    fixture.release(outcome)
}

/// Warm the private engine cache `cache` as the native mise fixture warms
/// its own (cospec change `installed-binary-template-warmup`): provision the
/// engine into it within `engine_warm_up_bound`, then build this build's
/// store template beside it with the test supervisor, and take the receipt.
/// Neither call takes the spawn gate, so the caller holds it across this.
async fn warm_fixture_cache(
    cache: &Path,
) -> Result<(PathBuf, crate::test_support::TemplateCacheReceipt)> {
    let started = Instant::now();
    let mut config = OpenOptions::new(PathBuf::new(), String::new()).config;
    config.offline = true;
    config.cache_dir = Some(cache.to_owned());
    let bound = crate::test_support::engine_warm_up_bound();
    let engine = tokio::time::timeout(bound, crate::provision::provision(&config, cache))
        .await
        .with_context(|| format!("provision the fixture engine cache exceeded {bound:?}"))??;
    crate::test_support::warm_template_cache(cache, &engine, &test_supervisor()?).await?;
    let receipt = crate::test_support::TemplateCacheReceipt::snapshot(cache, started.elapsed())?;
    Ok((engine, receipt))
}

/// A fixture that warms its own empty engine cache (the engine, then the
/// store template with a supervisor of this build) lets the first fresh
/// open in that cache, on the product path (no `template_root`), copy the
/// template with two engine starts instead of building it. The warm-up's
/// build is the template root's only engine start, a second warm-up and the
/// open leave its receipt valid, and the store records this build's key.
///
/// The start counts alone do not prove the open copied the warm-up's
/// template rather than building one: had the warm-up left no template and
/// the open built it, the template root would still show one build start.
/// That pin is carried by the receipt: `snapshot` fails unless the warm-up
/// itself published this build's template, and `verify_used` fails if the
/// open then built, republished, quarantined or re-keyed anything there.
#[tokio::test]
async fn fixture_cache_warm_up_lets_a_fresh_open_copy_with_two_starts() -> Result<()> {
    let fixture = fixture()?;
    let outcome = async {
        let cache = fixture.path().join("cache");
        // Warmed before the spawn gate held across the warm-up and the open.
        let mut options = options(fixture.path().join("data"), &scope('e'), None).await?;
        options.config.cache_dir = Some(cache.clone());
        ensure!(
            options.template_root.is_none() && !cache.exists(),
            "the fixture cache is not cold or the open is not on the product path"
        );
        let _gate = crate::spawn_gate::spawning().await;
        let (engine, receipt) = warm_fixture_cache(&cache).await?;
        ensure!(
            engine.starts_with(fs::canonicalize(&cache)?),
            "the engine was not provisioned into the fixture cache: {}",
            engine.display()
        );
        let root = root_in(&fs::canonicalize(&cache)?);
        crate::test_support::warm_template_cache(&cache, &engine, &test_supervisor()?).await?;
        receipt
            .verify_used()
            .context("a second warm-up changed the warmed cache")?;
        let store = MemoryStore::open(options.clone()).await?;
        let checked = async {
            let starts = starts_under(&options.data_dir)?;
            ensure!(
                starts == u64::from(FreshOpen::Template.starts()),
                "the open after the warm-up made {starts} engine starts"
            );
            ensure!(
                starts_under(&root)? == 1,
                "the template root saw other than the warm-up's one build start"
            );
            assert_template_born(&options, &store).await?;
            let recorded =
                crate::test_support::store_template_key(&options.data_dir, &options.project_scope)?;
            ensure!(
                crate::test_support::template_key() == key()
                    && recorded.as_deref() == Some(key())
                    && template_of(&options)? == recorded,
                "the store records template {recorded:?}, not this build's {}",
                key()
            );
            receipt
                .verify_used()
                .context("the open did not use the warmed template")
        }
        .await;
        store.close().await?;
        checked
    }
    .await;
    fixture.release(outcome)
}

/// The warmed cache's receipt names what changed in its templates root:
/// another key's lock is another template key, a build stage is a build,
/// and a template moved to `.rejected-*` is a quarantine with the published
/// template missing. Lock files are planted only in this private root.
#[tokio::test]
async fn fixture_cache_receipt_names_what_changed() -> Result<()> {
    let fixture = fixture()?;
    let outcome = async {
        let cache = fixture.path().join("cache");
        let receipt = {
            let _gate = crate::spawn_gate::spawning().await;
            warm_fixture_cache(&cache).await?.1
        };
        receipt.verify_used()?;
        ensure!(
            receipt.to_string().contains(key()),
            "the receipt does not name its key: {receipt}"
        );
        let root = root_in(&fs::canonicalize(&cache)?);
        let changed = |planted: &str| -> Result<String> {
            let error = receipt
                .verify_used()
                .err()
                .with_context(|| format!("the receipt accepted {planted}"))?;
            Ok(format!("{error:#}"))
        };
        let other = format!("{}.lock", "a".repeat(64));
        fs::write(root.join(&other), b"")?;
        let text = changed(&other)?;
        ensure!(
            text.contains(&format!("{other}: another template key")),
            "{text}"
        );
        fs::remove_file(root.join(&other))?;
        receipt.verify_used()?;
        let build = format!(".build-{}-planted", key());
        fs::create_dir(root.join(&build))?;
        let text = changed(&build)?;
        ensure!(text.contains(&format!("{build}: built")), "{text}");
        fs::remove_dir(root.join(&build))?;
        receipt.verify_used()?;
        // A republished template keeps its name, so no entry is added or
        // missing: only the published directory's identity tells.
        fn copy_tree(source: &Path, target: &Path) -> Result<()> {
            files::private_dir(target)?;
            for entry in fs::read_dir(source)? {
                let entry = entry?;
                let destination = target.join(entry.file_name());
                if entry.file_type()?.is_dir() {
                    copy_tree(&entry.path(), &destination)?;
                } else {
                    fs::copy(entry.path(), destination)?;
                }
            }
            Ok(())
        }
        let copy = root.join(format!(".copy-{}-planted", key()));
        copy_tree(&root.join(key()), &copy)?;
        fs::remove_dir_all(root.join(key()))?;
        fs::rename(&copy, root.join(key()))?;
        let text = changed("a republished template")?;
        ensure!(
            text.contains(&format!("{}: republished, so built", key()))
                && !text.contains("is missing"),
            "{text}"
        );
        let rejected = format!(".rejected-{}-planted", key());
        fs::rename(root.join(key()), root.join(&rejected))?;
        let text = changed(&rejected)?;
        ensure!(
            text.contains(&format!("{rejected}: quarantined"))
                && text.contains(&format!("{} is missing", key())),
            "{text}"
        );
        Ok(())
    }
    .await;
    fixture.release(outcome)
}

/// T5, design 3.9: while the first new project is inside its template build,
/// a second new project under the same key does not wait for it. It completes
/// on the cold path with the cold start count, reads no unpublished name and
/// leaves the template root as it found it; the first then publishes and
/// completes with three starts.
#[tokio::test]
async fn concurrent_new_projects_never_wait_for_a_template_build() -> Result<()> {
    let fixture = fixture()?;
    let outcome = async {
        let root = fixture.path().join("templates");
        // Both options are warmed before the one spawn gate both opens run
        // under: the first holds it through its pause, and a nested gate
        // behind a queued writer would wait for the first to resume.
        let first = options(fixture.path().join("first"), &scope('5'), Some(&root)).await?;
        let second = options(fixture.path().join("second"), &scope('6'), Some(&root)).await?;
        let pause = Arc::new(Pause::default());
        let hooks = Hooks {
            pause: Some(pause.clone()),
            ..Hooks::default()
        };
        let _gate = crate::spawn_gate::spawning().await;
        let building = tokio::spawn(HOOKS.scope(hooks, MemoryStore::open(first.clone())));
        let bound = crate::test_support::fresh_open_budget_of(FreshOpen::FirstProject);
        if tokio::time::timeout(bound, pause.reached.notified())
            .await
            .is_err()
        {
            building.abort();
            let _ = building.await;
            bail!("the first project did not reach its template build's pause in {bound:?}");
        }
        let during = entries_of(&root)?;
        let concurrent = async {
            ensure!(
                during
                    .iter()
                    .any(|name| name.starts_with(&format!(".build-{}-", key())))
                    && published(&root).is_none(),
                "the first project is not inside its build: {during:?}"
            );
            let store = MemoryStore::open(second.clone()).await?;
            let checked = async {
                let starts = starts_under(&second.data_dir)?;
                ensure!(
                    starts == u64::from(FreshOpen::Cold.starts()),
                    "the concurrent project made {starts} engine starts"
                );
                assert_cold(&second, &store).await?;
                ensure!(
                    entries_of(&root)? == during && interrupted(&second)?.is_empty(),
                    "the concurrent project touched the template root or a stage"
                );
                Ok(())
            }
            .await;
            store.close().await?;
            checked
        }
        .await;
        // Resume the first project whatever the second found.
        pause.resume.notify_one();
        let built = tokio::time::timeout(bound, building)
            .await
            .context("the first project did not finish after its pause")??;
        let store = built?;
        let checked = async {
            concurrent?;
            let starts = starts_under(fixture.path())? - starts_under(&second.data_dir)?;
            ensure!(
                starts == u64::from(FreshOpen::FirstProject.starts()),
                "the building project made {starts} engine starts"
            );
            assert_template_born(&first, &store).await?;
            ensure!(published(&root).is_some(), "the build published nothing");
            Ok(())
        }
        .await;
        store.close().await?;
        checked
    }
    .await;
    fixture.release(outcome)
}

/// T11: every file and directory a project's copy writes is synced before
/// the stage's identity record, which makes the copy adoptable, is written.
#[tokio::test]
async fn copied_files_and_directories_are_synced_before_identity() -> Result<()> {
    let fixture = fixture()?;
    let outcome = async {
        let options = options(fixture.path().join("project"), &scope('7'), None).await?;
        let shared = crate::test_support::shared_template_root()?;
        let manifest = read_manifest(&shared.join(key()))?;
        let events = Arc::new(StdMutex::new(Vec::new()));
        let hooks = Hooks {
            events: Some(events.clone()),
            ..Hooks::default()
        };
        let store = HOOKS.scope(hooks, open(options.clone())).await?;
        store.close().await?;
        let events = events.lock().expect("hook events").clone();
        let [identity] = events
            .iter()
            .enumerate()
            .filter_map(|(index, event)| match event {
                Event::Identity(stage) => Some((index, stage.clone())),
                _ => None,
            })
            .collect::<Vec<_>>()
            .try_into()
            .map_err(|found: Vec<_>| anyhow::anyhow!("identity writes: {found:?}"))?;
        let (at, stage) = identity;
        // Paths within the stage, after its own unique `.staging-<uuid>`
        // name. The copy writes through a checked handle whose path form
        // differs from the worker's (canonical on Unix, and neither the
        // verbatim `\\?\` form nor the configured form on Windows), and the
        // stage itself has since moved onto the active path, so no prefix
        // comparison can match them.
        let name = stage.file_name().context("stage has no name")?.to_owned();
        let within = |path: &Path| -> Option<PathBuf> {
            let mut components = path.components();
            components
                .by_ref()
                .find(|component| component.as_os_str() == name)?;
            Some(components.as_path().to_owned())
        };
        let synced = |range: &[Event]| -> BTreeSet<PathBuf> {
            range
                .iter()
                .filter_map(|event| match event {
                    Event::Synced(path) => within(path),
                    _ => None,
                })
                .collect()
        };
        let mut expected: BTreeSet<PathBuf> = manifest
            .entries
            .iter()
            .map(|entry| {
                let (Entry::Directory { path } | Entry::File { path, .. }) = entry;
                path.iter()
                    .fold(PathBuf::from(DATA), |parent, name| parent.join(name))
            })
            .collect();
        expected.insert(PathBuf::from(DATA));
        expected.insert(PathBuf::new());
        let before = synced(&events[..at]);
        ensure!(
            before == expected,
            "synced before the identity record: {before:?}; expected {expected:?}"
        );
        ensure!(
            synced(&events[at..]).is_empty(),
            "the copy synced after the identity record"
        );
        Ok(())
    }
    .await;
    fixture.release(outcome)
}

/// T9: an ordinary template-born store, opened fresh, written, used through
/// a promoted candidate, the usage ledger and an export, then reopened
/// writable and read-only, never pools a template-era revision: those carry
/// the placeholder identity.
#[tokio::test]
async fn ordinary_open_never_pools_a_pre_adoption_revision() -> Result<()> {
    let fixture = fixture()?;
    let outcome = async {
        let mut options = options(fixture.path().join("project"), &scope('8'), None).await?;
        let store = open(options.clone()).await?;
        let used = async {
            let pre_adoption = pre_adoption_commits(&store).await?;
            store.append("t9/main", "user", "on main").await?;
            let candidate = store.begin_candidate("t9").await?;
            candidate
                .view()
                .append("t9/candidate", "assistant", "on a candidate")
                .await?;
            candidate.promote().await?;
            store.usage_ledger()?.mark_new_session("t9-session").await?;
            let export = store.begin_active_export().await?;
            export.page(None).await?;
            let pooled = template_era_pools(&store, &pre_adoption);
            ensure!(pooled.is_empty(), "the fresh open pooled {pooled:?}");
            Ok(pre_adoption)
        }
        .await;
        store.close().await?;
        let pre_adoption = used?;
        for read_only in [false, true] {
            options.read_only = read_only;
            let store = open(options.clone()).await?;
            let pooled = template_era_pools(&store, &pre_adoption);
            store.close().await?;
            ensure!(
                pooled.is_empty(),
                "a reopen (read-only: {read_only}) pooled {pooled:?}"
            );
        }
        Ok(())
    }
    .await;
    fixture.release(outcome)
}

/// T15: a legacy import and a configured engine binary keep the cold staged
/// build, with its two starts, and never open the template root.
#[tokio::test]
async fn legacy_import_and_configured_binary_take_cold_path() -> Result<()> {
    let fixture = fixture()?;
    let outcome = async {
        let root = fixture.path().join("templates");

        let mut configured =
            options(fixture.path().join("configured"), &scope('9'), Some(&root)).await?;
        configured.config.dolt_binary = Some(crate::test_support::warm_runtime_cache().await?);
        let store = open(configured.clone()).await?;
        let checked = assert_cold(&configured, &store).await;
        store.close().await?;
        checked?;
        let starts = starts_under(&configured.data_dir)?;
        ensure!(
            starts == u64::from(FreshOpen::Cold.starts()),
            "a configured engine binary's open made {starts} engine starts"
        );

        let legacy_dir = fixture.path().join("legacy");
        files::private_dir(&legacy_dir)?;
        let legacy_scope = scope('a');
        let source = rusqlite::Connection::open(legacy_dir.join("memory.sqlite3"))?;
        source.execute_batch(
            "PRAGMA application_id=1263882837;
             PRAGMA user_version=1;
             CREATE TABLE messages (sequence INTEGER PRIMARY KEY AUTOINCREMENT, namespace TEXT NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL);
             CREATE TABLE state (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);",
        )?;
        source.execute(
            "INSERT INTO messages (namespace, role, content) VALUES (?1, 'user', 'legacy')",
            [format!("{legacy_scope}/transcript")],
        )?;
        drop(source);
        let legacy = options(legacy_dir, &legacy_scope, Some(&root)).await?;
        let store = open(legacy.clone()).await?;
        let checked = async {
            ensure!(
                template_of(&legacy)?.is_none(),
                "an imported store was created from a template"
            );
            ensure!(
                read_activation(&active(&legacy)?, &legacy_scope)?
                    .migration
                    .is_some(),
                "the legacy store was not imported"
            );
            Ok(())
        }
        .await;
        store.close().await?;
        checked?;
        let starts = starts_under(&legacy.data_dir)?;
        ensure!(
            starts == u64::from(FreshOpen::Cold.starts()),
            "a legacy import's open made {starts} engine starts"
        );
        ensure!(
            !root.exists(),
            "a cold-only open created the template root"
        );
        Ok(())
    }
    .await;
    fixture.release(outcome)
}

/// T22 and the busy cases through an ordinary open: an injected error
/// opening, locking or verifying the key lock, a key lock another holder
/// excludes, and a manifest read error each complete the open on the cold
/// path, with no error and no change to the published template.
#[tokio::test]
async fn template_lock_errors_and_busy_locks_send_the_opener_cold() -> Result<()> {
    let fixture = fixture()?;
    let outcome = async {
        let root = fixture.path().join("templates");
        let template = clone_shared(&root).await?;
        let faults = [
            (
                "lock open",
                Hooks {
                    lock: Some(LockStep::Open),
                    ..Hooks::default()
                },
            ),
            (
                "try-lock",
                Hooks {
                    lock: Some(LockStep::TryLock),
                    ..Hooks::default()
                },
            ),
            (
                "lock verification",
                Hooks {
                    lock: Some(LockStep::Verify),
                    ..Hooks::default()
                },
            ),
            (
                "manifest read",
                Hooks {
                    manifest_read_error: true,
                    ..Hooks::default()
                },
            ),
            ("busy", Hooks::default()),
        ];
        for (index, (label, hooks)) in faults.into_iter().enumerate() {
            let digit = char::from_digit(u32::try_from(index)? + 1, 16).context("digit")?;
            let options = options(
                fixture.path().join(format!("cold-{index}")),
                &scope(digit),
                Some(&root),
            )
            .await?;
            let held = match label {
                "busy" => Some(hold(&root, Mode::Exclusive).await?),
                _ => None,
            };
            let opened = HOOKS.scope(hooks, open(options.clone())).await;
            if let Some(held) = held {
                release(held).await;
            }
            let store = opened.with_context(|| format!("{label}: the open failed"))?;
            let checked = assert_cold(&options, &store).await;
            store.close().await?;
            checked.with_context(|| label.to_owned())?;
            let starts = starts_under(&options.data_dir)?;
            ensure!(
                starts == u64::from(FreshOpen::Cold.starts()),
                "{label}: the open made {starts} engine starts"
            );
            ensure!(
                published(&root) == Some(template) && rejected(&root)?.is_empty(),
                "{label}: the published template changed"
            );
            ensure!(
                interrupted(&options)?.is_empty(),
                "{label}: the open preserved a stage"
            );
        }
        Ok(())
    }
    .await;
    fixture.release(outcome)
}

/// T7 and T8 through an ordinary open. A structural verdict quarantines the
/// judged template before any copy; a digest mismatch mid-copy preserves the
/// copy remnant under `interrupted/` without an engine start and quarantines;
/// an injected read error mid-copy preserves the remnant and quarantines
/// nothing. Each open completes on the cold path.
#[tokio::test]
async fn damaged_templates_send_the_opener_cold_and_preserve_copy_remnants() -> Result<()> {
    let fixture = fixture()?;
    let outcome = async {
        let cases = ["structure", "digest", "read error"];
        for (index, label) in cases.into_iter().enumerate() {
            let root = fixture.path().join(format!("templates-{index}"));
            let template = clone_shared(&root).await?;
            let mut hooks = Hooks::default();
            match label {
                "structure" => {
                    let mut manifest = read_manifest(&root.join(key()))?;
                    manifest.format += 1;
                    write_manifest(&root.join(key()), &manifest)?;
                }
                "digest" => {
                    let file = largest_file(&root.join(key()))?;
                    let mut bytes = fs::read(&file)?;
                    let last = bytes.last_mut().context("empty template file")?;
                    *last ^= 1;
                    files::write(&file, &bytes)?;
                }
                _ => {
                    let file = largest_file(&root.join(key()))?;
                    let name = file
                        .file_name()
                        .context("template file has no name")?
                        .to_string_lossy()
                        .into_owned();
                    hooks.read = Some(ReadFault::new(&name));
                }
            }
            let digit = char::from_digit(u32::try_from(index)? + 1, 16).context("digit")?;
            let options = options(
                fixture.path().join(format!("project-{index}")),
                &scope(digit),
                Some(&root),
            )
            .await?;
            let store = HOOKS
                .scope(hooks, open(options.clone()))
                .await
                .with_context(|| format!("{label}: the open failed"))?;
            let checked = assert_cold(&options, &store).await;
            store.close().await?;
            checked.with_context(|| label.to_owned())?;
            let quarantined = rejected(&root)?;
            let remnants = interrupted(&options)?;
            match label {
                "read error" => ensure!(
                    published(&root) == Some(template) && quarantined.is_empty(),
                    "{label}: the template was quarantined"
                ),
                _ => ensure!(
                    published(&root).is_none() && quarantined.len() == 1,
                    "{label}: the template was not quarantined: {quarantined:?}"
                ),
            }
            match label {
                "structure" => ensure!(
                    remnants.is_empty(),
                    "{label}: a stage was copied before the structural check"
                ),
                _ => {
                    let [remnant] = remnants.as_slice() else {
                        bail!("{label}: not exactly one preserved remnant: {remnants:?}");
                    };
                    // Its lease came from the preservation's quiescence wait,
                    // not from a reap this process recorded.
                    let lifecycles =
                        cfg!(windows).then(|| options.data_dir.join("memory/lifecycles"));
                    crate::test_support::await_store_quiescence(remnant, lifecycles.as_deref())
                        .await?;
                    let key = engine_ledger::key(remnant).context("no ledger key")?;
                    ensure!(
                        remnant.join(DATA).is_dir()
                            && !remnant.join("identity.json").exists()
                            && engine_ledger::with(|ledger| ledger.starts(&key)) == 0,
                        "{label}: the remnant {} is not an unstarted copy",
                        remnant.display()
                    );
                }
            }
            let starts = starts_under(&options.data_dir)?;
            ensure!(
                starts == u64::from(FreshOpen::Cold.starts()),
                "{label}: the open made {starts} engine starts"
            );
        }
        Ok(())
    }
    .await;
    fixture.release(outcome)
}

/// Adoption and the template shape run on the copy's own engine: a verdict
/// there fails the open with the typed verdict and no retry, preserves the
/// unready stage and quarantines the template it was copied from. The next
/// new project builds a template again.
#[tokio::test]
async fn shape_verdict_on_the_copy_fails_the_open_and_quarantines_the_template() -> Result<()> {
    let fixture = fixture()?;
    let outcome = async {
        let root = fixture.path().join("templates");
        let engine = engine().await?;
        // A template whose bytes pass the build's own shape check and every
        // structural and byte check, but hold a branch outside the compiled
        // set: a copy's shape check refuses it.
        let hooks = Hooks {
            after_shape: vec!["CALL DOLT_BRANCH('extra_branch')".to_owned()],
            ..Hooks::default()
        };
        let built = HOOKS
            .scope(hooks, ensure(&root, &engine))
            .await
            .map_err(failure)?;
        ensure!(matches!(built, Ensured::Built(_)), "{built:?}");
        let flawed = published(&root).context("the flawed template was not published")?;
        let options = options(fixture.path().join("project"), &scope('b'), Some(&root)).await?;
        let error = open(options.clone())
            .await
            .err()
            .context("a copy of a flawed template was activated")?;
        let message = format!("{error:#}");
        ensure!(
            TemplateVerdict::find(&error).is_some() && message.contains("unexpected branch"),
            "the open did not fail with the shape verdict: {message}"
        );
        ensure!(!active(&options)?.exists(), "an active directory appeared");
        let preserved = interrupted(&options)?;
        let [stage] = preserved.as_slice() else {
            bail!("not exactly one preserved stage: {preserved:?}");
        };
        ensure!(
            !stage.join("ready.json").exists()
                && read_identity_view(stage)?.template.as_deref() == Some(key()),
            "the preserved stage is not the unready copy"
        );
        let quarantined = rejected(&root)?;
        let [moved] = quarantined.as_slice() else {
            bail!("not exactly one quarantined template: {quarantined:?}");
        };
        ensure!(
            published(&root).is_none() && files::directory(&root.join(moved))?.identity() == flawed,
            "the judged template was not the one quarantined"
        );
        ensure!(
            starts_under(&options.data_dir)? == 1,
            "the failed open retried or reached its active start"
        );
        // The next new project pays one build, then copies the new template.
        let next = options_for(&fixture, &root, 'c').await?;
        let store = open(next.clone()).await?;
        let checked = assert_template_born(&next, &store).await;
        store.close().await?;
        checked?;
        let starts = starts_under(&next.data_dir)?;
        ensure!(
            starts == u64::from(FreshOpen::Template.starts()) && published(&root).is_some(),
            "the next project made {starts} starts in its own directory"
        );
        Ok(())
    }
    .await;
    fixture.release(outcome)
}

async fn options_for(fixture: &TempDir, root: &Path, digit: char) -> Result<OpenOptions> {
    options(
        fixture.path().join(format!("project-{digit}")),
        &scope(digit),
        Some(root),
    )
    .await
}

/// T20 through an ordinary open: a failure on the copy's own engine that is
/// not a verdict (here the stage's ready-marker observer closing before
/// release) fails the open, preserves the unready stage and leaves the
/// published template exactly as it was; the next new project copies it
/// with two starts.
#[tokio::test]
async fn engine_failure_on_the_copy_preserves_the_stage_and_keeps_the_template() -> Result<()> {
    let fixture = fixture()?;
    let outcome = async {
        let root = fixture.path().join("templates");
        let template = clone_shared(&root).await?;
        let options = options_for(&fixture, &root, 'd').await?;
        let (observation, release_marker, opening) =
            marker_fixture::prepare(options.clone(), false);
        let opening = tokio::spawn(async move {
            let _gate = crate::spawn_gate::spawning().await;
            opening.await
        });
        let bound = crate::test_support::fresh_open_budget_of(FreshOpen::Template);
        let observed = tokio::time::timeout(bound, observation)
            .await
            .context("the copy did not reach its ready marker")?
            .context("the copy ended before its ready marker")?;
        ensure!(!observed.after_marker);
        drop(release_marker);
        let error = tokio::time::timeout(bound, opening)
            .await
            .context("the failed open did not return")??
            .err()
            .context("the open succeeded without its ready marker")?;
        ensure!(
            TemplateVerdict::find(&error).is_none(),
            "an engine-side failure was a verdict: {error:#}"
        );
        let preserved = interrupted(&options)?;
        ensure!(
            preserved.len() == 1 && !preserved[0].join("ready.json").exists(),
            "the unready stage was not preserved: {preserved:?}"
        );
        ensure!(
            published(&root) == Some(template) && rejected(&root)?.is_empty(),
            "an engine-side failure changed the template"
        );
        let next = options_for(&fixture, &root, 'e').await?;
        let store = open(next.clone()).await?;
        let checked = assert_template_born(&next, &store).await;
        store.close().await?;
        checked?;
        let starts = starts_under(&next.data_dir)?;
        ensure!(
            starts == u64::from(FreshOpen::Template.starts()),
            "the next project made {starts} engine starts"
        );
        Ok(())
    }
    .await;
    fixture.release(outcome)
}

/// A template build the open started fails the open with the build's error
/// and is never followed by the cold path: here the build's own shape check
/// refuses a branch outside the compiled set. The build engine is the only
/// start, nothing is published or quarantined, no store appears at the
/// project's path and no stage is left behind.
#[tokio::test]
async fn failed_template_build_fails_the_open_without_a_cold_retry() -> Result<()> {
    let fixture = fixture()?;
    let outcome = async {
        let root = fixture.path().join("templates");
        let options = options_for(&fixture, &root, '1').await?;
        let hooks = Hooks {
            before_shape: vec!["CALL DOLT_BRANCH('extra_branch')".to_owned()],
            ..Hooks::default()
        };
        let error = HOOKS
            .scope(hooks, open(options.clone()))
            .await
            .err()
            .context("an open whose template build was refused succeeded")?;
        let message = format!("{error:#}");
        ensure!(
            message.contains("build the memory store template")
                && message.contains("unexpected branch"),
            "the open did not fail with the build's own refusal: {message}"
        );
        let starts = starts_under(fixture.path())?;
        ensure!(
            starts == 1 && starts_under(&root)? == 1,
            "the failed build was followed by {} more engine starts",
            starts.saturating_sub(1)
        );
        ensure!(
            published(&root).is_none() && rejected(&root)?.is_empty(),
            "a refused build published or quarantined a template"
        );
        ensure!(!active(&options)?.exists(), "an active directory appeared");
        ensure!(
            stages(&options)?.is_empty() && interrupted(&options)?.is_empty(),
            "the failed open left a stage"
        );
        Ok(())
    }
    .await;
    fixture.release(outcome)
}

/// The copy a first launch makes from the template its own open just built
/// fails the open like the build would, with no cold retry: the chain ran
/// once and the build engine is the open's only start. A read error leaves
/// the new template published for the next project, which copies it in two
/// starts; a byte verdict moves it aside under the key lock the build still
/// holds, and the next project builds again. Either way the copy remnant is
/// preserved under `interrupted/` without an engine start.
#[tokio::test]
async fn failed_copy_after_the_build_fails_the_open_without_a_cold_retry() -> Result<()> {
    let fixture = fixture()?;
    let outcome = async {
        for (index, verdict) in [false, true].into_iter().enumerate() {
            let label = if verdict { "verdict" } else { "read error" };
            let root = fixture.path().join(format!("templates-{index}"));
            let digit = char::from_digit(u32::try_from(index)? + 3, 16).context("digit")?;
            let options = options_for(&fixture, &root, digit).await?;
            let fault = ReadFault::published(key(), verdict);
            let hooks = Hooks {
                read: Some(fault.clone()),
                ..Hooks::default()
            };
            let error = HOOKS
                .scope(hooks, open(options.clone()))
                .await
                .err()
                .with_context(|| format!("{label}: an open whose copy failed succeeded"))?;
            let message = format!("{error:#}");
            ensure!(
                fault.fired()
                    && message.contains(
                        "copy the new project from the memory store template this open built"
                    )
                    && TemplateVerdict::find(&error).is_some() == verdict,
                "{label}: the open did not fail with its copy's failure: {message}"
            );
            // The build succeeded and published: the error names the copy,
            // never a failed build.
            ensure!(
                !message.contains("build the memory store template")
                    && !message.contains("template build failed"),
                "{label}: a failed copy was reported as a failed build: {message}"
            );
            let starts = starts_under(&options.data_dir)?;
            ensure!(
                starts == 0 && starts_under(&root)? == 1,
                "{label}: the failed copy was followed by {starts} more engine starts"
            );
            if verdict {
                ensure!(
                    published(&root).is_none() && rejected(&root)?.len() == 1,
                    "{label}: the template this open built was not quarantined"
                );
            } else {
                ensure!(
                    published(&root).is_some() && rejected(&root)?.is_empty(),
                    "{label}: a read error changed the template this open built"
                );
            }
            ensure!(
                !active(&options)?.exists(),
                "{label}: an active directory appeared"
            );
            ensure!(
                stages(&options)?.is_empty(),
                "{label}: the failed open left a stage beside its active path"
            );
            let remnants = interrupted(&options)?;
            let [remnant] = remnants.as_slice() else {
                bail!("{label}: not exactly one preserved remnant: {remnants:?}");
            };
            let lifecycles = cfg!(windows).then(|| options.data_dir.join("memory/lifecycles"));
            crate::test_support::await_store_quiescence(remnant, lifecycles.as_deref()).await?;
            let remnant_key = engine_ledger::key(remnant).context("no ledger key")?;
            ensure!(
                remnant.join(DATA).is_dir()
                    && !remnant.join("identity.json").exists()
                    && engine_ledger::with(|ledger| ledger.starts(&remnant_key)) == 0,
                "{label}: the remnant {} is not an unstarted copy",
                remnant.display()
            );
            // The next new project copies the intact template, or builds
            // the quarantined one again; either way it is template-born.
            let next_digit = char::from_digit(u32::try_from(index)? + 5, 16).context("digit")?;
            let next = options_for(&fixture, &root, next_digit).await?;
            let store = open(next.clone()).await?;
            let checked = assert_template_born(&next, &store).await;
            store.close().await?;
            checked.with_context(|| format!("{label}: the next project"))?;
            let next_starts = starts_under(&next.data_dir)?;
            let builds = starts_under(&root)?;
            let expected = if verdict { 2 } else { 1 };
            ensure!(
                next_starts == u64::from(FreshOpen::Template.starts())
                    && builds == expected
                    && published(&root).is_some(),
                "{label}: the next project made {next_starts} starts in its directory and \
                 {builds} template builds ran in all"
            );
        }
        Ok(())
    }
    .await;
    fixture.release(outcome)
}

/// While a first launch builds the template inside its open, the creating
/// stage has already been reported, and no stage that changes the terminal's
/// sentence follows it: the whole template path runs under "Creating this
/// project's memory…". This is the memory-side guarantee the terminal
/// tests show on a real PTY, and it runs on every operating system.
#[tokio::test]
async fn first_project_reports_creation_before_its_template_build() -> Result<()> {
    let fixture = fixture()?;
    let outcome = async {
        let root = fixture.path().join("templates");
        let options = options_for(&fixture, &root, '2').await?;
        let pause = Arc::new(Pause::default());
        let hooks = Hooks {
            pause: Some(pause.clone()),
            ..Hooks::default()
        };
        let (mut progress, opening) = MemoryStore::open_observed(options.clone());
        let _gate = crate::spawn_gate::spawning().await;
        let building = tokio::spawn(HOOKS.scope(hooks, opening));
        let bound = crate::test_support::fresh_open_budget_of(FreshOpen::FirstProject);
        if tokio::time::timeout(bound, pause.reached.notified())
            .await
            .is_err()
        {
            building.abort();
            let _ = building.await;
            bail!("the first project did not reach its template build's pause in {bound:?}");
        }
        // Every stage reported so far is already queued: the build is held.
        let mut during = Vec::new();
        while let Ok(Some(stage)) =
            tokio::time::timeout(Duration::from_millis(100), progress.recv()).await
        {
            during.push(stage);
        }
        let building_now = entries_of(&root)?
            .iter()
            .any(|name| name.starts_with(&format!(".build-{}-", key())));
        pause.resume.notify_one();
        let store = tokio::time::timeout(bound, building)
            .await
            .context("the first project did not finish after its pause")???;
        let mut stages = during.clone();
        while let Some(stage) = progress.recv().await {
            stages.push(stage);
        }
        let checked = async {
            ensure!(building_now, "the open was not inside its template build");
            ensure!(
                during.contains(&crate::MemoryOpenStage::CreatingDatabase),
                "the template build began before the creating stage: {during:?}"
            );
            let created = stages
                .iter()
                .position(|stage| *stage == crate::MemoryOpenStage::CreatingDatabase)
                .context("no creating stage")?;
            let after = &stages[created + 1..];
            ensure!(
                after.iter().all(|stage| matches!(
                    stage,
                    crate::MemoryOpenStage::OpeningDatabase | crate::MemoryOpenStage::Ready
                )) && after.last() == Some(&crate::MemoryOpenStage::Ready),
                "a stage after creation could change the sentence: {stages:?}"
            );
            assert_template_born(&options, &store).await
        }
        .await;
        store.close().await?;
        checked
    }
    .await;
    fixture.release(outcome)
}

/// A failure of the copy's own engine start that is not a verdict (here the
/// supervisor refusing a stage that names another build's template key)
/// fails the open with an ordinary error naming both keys and leaves the
/// template untouched. The unready stage stays in its staging directory; the
/// next open of the project preserves it under `interrupted/` without
/// starting its engine and copies the template again, in two starts.
#[tokio::test]
async fn failed_stage_start_leaves_the_copy_for_the_next_open_to_preserve() -> Result<()> {
    let fixture = fixture()?;
    let outcome = async {
        let root = fixture.path().join("templates");
        let template = clone_shared(&root).await?;
        let options = options_for(&fixture, &root, '3').await?;
        let foreign = "a-template-key-from-another-build";
        let hooks = Hooks {
            stage_key: Some(foreign.to_owned()),
            ..Hooks::default()
        };
        let error = HOOKS
            .scope(hooks, open(options.clone()))
            .await
            .err()
            .context("a stage naming another build's key was adopted")?;
        let message = format!("{error:#}");
        ensure!(
            TemplateVerdict::find(&error).is_none()
                && message.contains(foreign)
                && message.contains(key()),
            "a key mismatch was a verdict or did not name both keys: {message}"
        );
        ensure!(
            published(&root) == Some(template) && rejected(&root)?.is_empty(),
            "a failed stage start changed the template"
        );
        ensure!(!active(&options)?.exists(), "an active directory appeared");
        ensure!(
            interrupted(&options)?.is_empty(),
            "the failed start preserved its stage itself"
        );
        let left = stages(&options)?;
        let [stage] = left.as_slice() else {
            bail!("not exactly one stage left in place: {left:?}");
        };
        let identity = read_identity_view(stage)?;
        ensure!(
            !identity.initialized
                && identity.template.as_deref() == Some(foreign)
                && !stage.join("ready.json").exists(),
            "the stage left in place is not the unready copy: {identity:?}"
        );
        let before = starts_under(&options.data_dir)?;
        let store = open(options.clone()).await?;
        let checked = async {
            let starts = starts_under(&options.data_dir)? - before;
            ensure!(
                starts == u64::from(FreshOpen::Template.starts()),
                "the next open made {starts} engine starts"
            );
            assert_template_born(&options, &store).await?;
            let preserved = interrupted(&options)?;
            let [remnant] = preserved.as_slice() else {
                bail!("the next open did not preserve one stage: {preserved:?}");
            };
            ensure!(
                read_identity_view(remnant)?.template.as_deref() == Some(foreign)
                    && stages(&options)?.is_empty(),
                "the preserved stage is not the refused copy"
            );
            Ok(())
        }
        .await;
        store.close().await?;
        checked
    }
    .await;
    fixture.release(outcome)
}

/// Adoption refuses a copy whose placeholder row on the usage branch is not
/// the compiled one before its engine serves: the open fails with the typed
/// verdict, the judged template is quarantined, and the unready stage stays
/// in its staging directory for the next open's recovery.
#[tokio::test]
async fn adoption_verdict_quarantines_and_leaves_the_stage_in_place() -> Result<()> {
    let fixture = fixture()?;
    let outcome = async {
        let root = fixture.path().join("templates");
        let engine = engine().await?;
        // Committed on the usage branch only after the build's own shape
        // check passed, so the template is published with it. `main` keeps
        // the placeholder row its pool's connections check.
        let hooks = Hooks {
            after_shape: vec![
                "USE `kuru/kuru_usage_v1`".to_owned(),
                "UPDATE kuru_instance SET instance_id = \
                 '11111111-1111-4111-8111-111111111111' WHERE singleton = 1"
                    .to_owned(),
                "CALL DOLT_COMMIT('-am', 'foreign usage identity', '--author', \
                 'Kuru <memory@kuru.local>')"
                    .to_owned(),
                "USE `kuru`".to_owned(),
            ],
            ..Hooks::default()
        };
        let built = HOOKS
            .scope(hooks, ensure(&root, &engine))
            .await
            .map_err(failure)?;
        ensure!(matches!(built, Ensured::Built(_)), "{built:?}");
        let flawed = published(&root).context("the flawed template was not published")?;
        let options = options_for(&fixture, &root, '4').await?;
        let error = open(options.clone())
            .await
            .err()
            .context("a copy with a foreign placeholder row was adopted")?;
        let message = format!("{error:#}");
        ensure!(
            TemplateVerdict::find(&error).is_some()
                && message.contains("is not the compiled template placeholder"),
            "the open did not fail with the adoption verdict: {message}"
        );
        let quarantined = rejected(&root)?;
        let [moved] = quarantined.as_slice() else {
            bail!("not exactly one quarantined template: {quarantined:?}");
        };
        ensure!(
            published(&root).is_none() && files::directory(&root.join(moved))?.identity() == flawed,
            "the judged template was not the one quarantined"
        );
        ensure!(!active(&options)?.exists(), "an active directory appeared");
        ensure!(
            interrupted(&options)?.is_empty(),
            "the refused start preserved its stage itself"
        );
        let left = stages(&options)?;
        let [stage] = left.as_slice() else {
            bail!("not exactly one stage left in place: {left:?}");
        };
        let identity = read_identity_view(stage)?;
        ensure!(
            !identity.initialized && identity.template.as_deref() == Some(key()),
            "the stage left in place is not the unready copy: {identity:?}"
        );
        Ok(())
    }
    .await;
    fixture.release(outcome)
}

const QUARANTINE_CHILD_TEST: &str = "store::creation_template::open_tests::child_process_fixture_open_quarantines_the_shared_template";

/// Runs only as a child of
/// `fixture_open_that_quarantines_the_shared_template_fails_teardown`, whose
/// environment names a private cache, holding a template with a damaged
/// manifest, as this process's shared test cache. The fixture's open
/// quarantines that shared template on its structural check and is created
/// cold, and the fixture root's release fails, naming the quarantine.
#[tokio::test]
async fn child_process_fixture_open_quarantines_the_shared_template() -> Result<()> {
    let Some(outcome) = std::env::var_os(CHILD_OUTCOME) else {
        return Ok(());
    };
    let cache = crate::store::test_cache();
    {
        let _gate = crate::spawn_gate::spawning().await;
        let config = OpenOptions::new(PathBuf::new(), String::new()).config;
        crate::provision::provision(&config, &cache).await?;
    }
    let root = crate::test_support::tempdir()?;
    let mut options = crate::test_support::open_options(root.path().join("project"), scope('9'))?;
    options.fixture = Some(Fixture::Warmed);
    let store = open(options.clone()).await?;
    let project = starts_under(&options.data_dir)?;
    let cold = template_of(&options)?.is_none();
    store.close().await?;
    let shared = crate::test_support::shared_template_root()?;
    let quarantined = rejected(&shared)?.len();
    let left = published(&shared).is_some();
    let verdict = match root.release(Ok(())) {
        Ok(()) => "passed".to_owned(),
        Err(error) => format!(
            "failed={}",
            format!("{error:#}").contains("quarantined the shared store template")
        ),
    };
    files::write(
        Path::new(&outcome),
        format!(
            "project={project} cold={cold} quarantined={quarantined} published={left} \
             guard={verdict}"
        )
        .as_bytes(),
    )
}

/// The class guard's quarantine trigger through a real open: a fixture whose
/// open quarantines the shared store template fails at teardown naming the
/// quarantine. A child process whose shared cache holds a damaged copy of
/// this process's template keeps this process's own template untouched.
#[tokio::test]
async fn fixture_open_that_quarantines_the_shared_template_fails_teardown() -> Result<()> {
    let fixture = fixture()?;
    let cache = fixture.path().join("cache");
    files::private_dir(&cache)?;
    let damaged = root_in(&fs::canonicalize(&cache)?);
    clone_shared(&damaged).await?;
    let mut manifest = read_manifest(&damaged.join(key()))?;
    manifest.format += 1;
    write_manifest(&damaged.join(key()), &manifest)?;
    let outcome = fixture.path().join("outcome");
    let log = fixture.path().join("child.log");
    let child = spawn_child(QUARANTINE_CHILD_TEST, &cache, &outcome, &log).await?;
    let status = wait_child(child).await?;
    let diagnostics = format!(
        "stdout: {}\nstderr: {}",
        fs::read_to_string(&log).unwrap_or_default(),
        fs::read_to_string(log.with_extension("stderr")).unwrap_or_default()
    );
    let checked = async {
        ensure!(status.success(), "child failed ({status}): {diagnostics}");
        let reported = String::from_utf8(files::read_bytes(&outcome, 256)?)?;
        ensure!(
            reported
                == format!(
                    "project={} cold=true quarantined=1 published=false guard=failed=true",
                    FreshOpen::Cold.starts()
                ),
            "{reported}; {diagnostics}"
        );
        Ok(())
    }
    .await;
    fixture.release(checked)
}

const GUARD_CHILD_TEST: &str =
    "store::creation_template::open_tests::child_process_fixture_open_builds_the_shared_template";

/// Runs only as a child of
/// `fixture_open_that_builds_the_shared_template_fails_teardown`, whose
/// environment names a private, empty cache as this process's shared test
/// cache. Its fixture options claim a warm-up that never built the template,
/// so the open builds the shared template itself (three starts), and the
/// fixture root's release fails, naming the build.
#[tokio::test]
async fn child_process_fixture_open_builds_the_shared_template() -> Result<()> {
    let Some(outcome) = std::env::var_os(CHILD_OUTCOME) else {
        return Ok(());
    };
    let cache = crate::store::test_cache();
    {
        let _gate = crate::spawn_gate::spawning().await;
        let config = OpenOptions::new(PathBuf::new(), String::new()).config;
        crate::provision::provision(&config, &cache).await?;
    }
    let root = crate::test_support::tempdir()?;
    let mut options = crate::test_support::open_options(root.path().join("project"), scope('f'))?;
    // As if warmed: the guard before the open passes, the template is absent.
    options.fixture = Some(Fixture::Warmed);
    let store = open(options.clone()).await?;
    let project = starts_under(&options.data_dir)?;
    let built = starts_under(&crate::test_support::shared_template_root()?)?;
    store.close().await?;
    let verdict = match root.release(Ok(())) {
        Ok(()) => "passed".to_owned(),
        Err(error) => format!(
            "failed={}",
            format!("{error:#}").contains("built the shared store template")
        ),
    };
    files::write(
        Path::new(&outcome),
        format!("project={project} build={built} guard={verdict}").as_bytes(),
    )
}

/// T18, the real trigger: a fixture whose open builds the shared store
/// template, because its options claim a warm-up that never ran, fails at
/// teardown naming the build. A child process with a private cache makes the
/// shared template absent without disturbing this process's.
#[tokio::test]
async fn fixture_open_that_builds_the_shared_template_fails_teardown() -> Result<()> {
    let fixture = fixture()?.allowing_template_build();
    let cache = fixture.path().join("cache");
    files::private_dir(&cache)?;
    let outcome = fixture.path().join("outcome");
    let log = fixture.path().join("child.log");
    let child = spawn_child(GUARD_CHILD_TEST, &cache, &outcome, &log).await?;
    let status = wait_child(child).await?;
    let diagnostics = format!(
        "stdout: {}\nstderr: {}",
        fs::read_to_string(&log).unwrap_or_default(),
        fs::read_to_string(log.with_extension("stderr")).unwrap_or_default()
    );
    let checked = async {
        ensure!(status.success(), "child failed ({status}): {diagnostics}");
        let reported = String::from_utf8(files::read_bytes(&outcome, 256)?)?;
        ensure!(
            reported == "project=2 build=1 guard=failed=true",
            "{reported}; {diagnostics}"
        );
        Ok(())
    }
    .await;
    fixture.release(checked)
}
