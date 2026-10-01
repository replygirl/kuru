//! Tests of the store template cache (cospec change `memory-template-cache`).
use super::hooks::{Event, HOOKS, Hooks, LockStep, Pause, ReadFault};
use super::*;
use crate::test_support::{TempDir, engine_ledger};
use std::collections::BTreeMap;

/// Template roots hold Dolt repositories the fixture guard reads in full:
/// `<root>/<key>/data/kuru/.dolt/stats/.dolt/noms`, and a child process's
/// engine cache one level deeper.
const DEPTH: usize = 16;
/// A bound for one operation that must not wait.
const PROMPT: Duration = Duration::from_secs(10);

/// A private fixture root for template cache tests, which build and
/// quarantine in their own private template roots.
fn fixture() -> Result<TempDir> {
    Ok(TempDir::new("kuru-template-cache-", None)?
        .with_depth_budget(DEPTH)
        .allowing_template_build())
}

fn startup() -> Duration {
    Duration::from_secs(
        OpenOptions::new(PathBuf::new(), String::new())
            .config
            .startup_timeout_secs,
    )
}

/// The warmed engine and the test supervisor.
async fn engine() -> Result<Engine> {
    Ok(Engine {
        binary: crate::test_support::warm_runtime_cache().await?,
        supervisor: test_supervisor()?,
        timeout: startup(),
    })
}

/// An engine for calls that must never start one.
fn no_engine() -> Engine {
    Engine {
        binary: PathBuf::from("/nonexistent/dolt"),
        supervisor: PathBuf::from("/nonexistent/supervisor"),
        timeout: startup(),
    }
}

fn failure(failure: CreationFailure) -> anyhow::Error {
    anyhow::Error::from(failure)
}

/// `ensure_in` without waiting, under the spawn gate (a build starts a
/// supervisor and Dolt).
async fn ensure(root: &Path, engine: &Engine) -> Result<Ensured, CreationFailure> {
    let _gate = crate::spawn_gate::spawning().await;
    ensure_in(root, engine, Wait::Never).await
}

/// `create_in` into `stage`, under the spawn gate.
async fn create(
    root: &Path,
    engine: &Engine,
    stage: &Directory,
) -> Result<Created, CreationFailure> {
    let _gate = crate::spawn_gate::spawning().await;
    create_in(root, engine, stage).await
}

/// A new private destination stage beneath the fixture.
fn stage(fixture: &TempDir, name: &str) -> Result<Directory> {
    files::ensure_private_directory(&fixture.path().join(name))
}

/// The sorted top-level names of a template root, without the Windows
/// lifecycle leases.
fn entries_of(root: &Path) -> Result<Vec<String>> {
    let mut names = fs::read_dir(root)?
        .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
        .collect::<Result<Vec<_>>>()?;
    names.retain(|name| name != "lifecycles");
    names.sort();
    Ok(names)
}

fn key() -> &'static str {
    compiled_key()
}

/// The key lock of `root`, taken in `mode` without waiting, under the lock
/// gate.
async fn hold(root: &Path, mode: Mode) -> Result<File> {
    let root = open_root(root)?;
    let _gate = crate::spawn_gate::locking_async().await;
    try_key_lock(&root, key(), mode)?.context("the key lock was busy")
}

/// Release a held key lock under the lock gate.
async fn release(lock: File) {
    let _gate = crate::spawn_gate::locking_async().await;
    drop(lock);
}

/// Publish under `root` a copy of the shared, warmed template, read under
/// its shared key lock, and return the copy's directory identity.
async fn clone_shared(root: &Path) -> Result<FileIdentity> {
    crate::test_support::warm_runtime_cache().await?;
    let shared = open_root(&crate::test_support::shared_template_root()?)?;
    let deadline = Instant::now() + Duration::from_secs(120);
    let lock = loop {
        let attempt = {
            let _gate = crate::spawn_gate::locking_async().await;
            try_key_lock(&shared, key(), Mode::Shared)?
        };
        if let Some(lock) = attempt {
            break lock;
        }
        ensure!(
            Instant::now() < deadline,
            "the shared template stayed locked"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    let copied = (|| -> Result<FileIdentity> {
        let Inspection::Valid(judged) = inspect(&shared, key()).map_err(failure)? else {
            bail!("the shared template is not published after warm-up");
        };
        let root = open_root(root)?;
        let template = root.create_private_directory(OsStr::new(key()))?;
        copy_into(&judged, &template).map_err(failure)?;
        let mut file = template.create_new(OsStr::new(MANIFEST))?;
        file.write_all(&serde_json::to_vec(&judged.manifest)?)?;
        file.sync_all()?;
        Ok(template.identity())
    })();
    release(lock).await;
    copied
}

/// Plant under `root`, for `key`, an abandoned build store (with no live
/// lease) and an abandoned capture stage, holding no handle on either, and
/// return their names.
fn plant_abandoned(root: &Path, key: &str) -> Result<[String; 2]> {
    let directory = open_root(root)?;
    let build = format!(".build-{key}-{}", Uuid::new_v4());
    directory
        .create_private_directory(OsStr::new(&build))?
        .create_private_directory(OsStr::new(BUILD_STORE))?
        .create_private_directory(OsStr::new(DATA))?;
    let stage = format!(".stage-{key}-{}", Uuid::new_v4());
    let partial = directory.create_private_directory(OsStr::new(&stage))?;
    files::write(&partial.path().join("partial"), b"a partial capture")?;
    Ok([build, stage])
}

/// The template directory's identity, if it is published.
fn published(root: &Path) -> Option<FileIdentity> {
    files::directory(&root.join(key()))
        .ok()
        .map(|template| template.identity())
}

/// This key's quarantined directories under `root`.
fn rejected(root: &Path) -> Result<Vec<String>> {
    let prefix = format!(".rejected-{}-", key());
    Ok(entries_of(root)?
        .into_iter()
        .filter(|name| name.starts_with(&prefix))
        .collect())
}

/// The manifest of the template directory `template`.
fn read_manifest(template: &Path) -> Result<Manifest> {
    Ok(serde_json::from_slice(&files::read_bytes(
        &template.join(MANIFEST),
        MANIFEST_LIMIT,
    )?)?)
}

/// Rewrite the manifest of the template directory `template`.
fn write_manifest(template: &Path, manifest: &Manifest) -> Result<()> {
    files::write(&template.join(MANIFEST), &serde_json::to_vec(manifest)?)
}

/// The largest data file of a template, by its manifest.
fn largest_file(template: &Path) -> Result<PathBuf> {
    let manifest = read_manifest(template)?;
    let (path, _) = manifest
        .entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::File { path, bytes, .. } => Some((path, *bytes)),
            Entry::Directory { .. } => None,
        })
        .max_by_key(|(_, bytes)| *bytes)
        .context("the template has no file")?;
    Ok(path
        .iter()
        .fold(template.join(DATA), |parent, name| parent.join(name)))
}

/// The key changes with each of its inputs, and with nothing else.
#[test]
fn template_key_tracks_schema_engine_statements_and_format_only() {
    let definitions = [
        vec!["v2".to_owned(), "v3".to_owned()],
        vec!["v2".to_owned()],
    ];
    let statements = ["CREATE TABLE a (b INT)", "Initialize"];
    let base = KeyInputs {
        format: 1,
        engine: "2.3.5",
        executable: "executable digest",
        target: "aarch64-apple-darwin",
        schema: 7,
        usage_schema: 4,
        definitions: &definitions,
        statements: &statements,
    };
    let edited_definition = [
        vec!["v2".to_owned(), "v3 edited".to_owned()],
        vec!["v2".to_owned()],
    ];
    let moved_definition = [
        vec!["v2".to_owned()],
        vec!["v3".to_owned(), "v2".to_owned()],
    ];
    let edited_statement = ["CREATE TABLE a (b BIGINT)", "Initialize"];
    let extra_statement = ["CREATE TABLE a (b INT)", "Initialize", "GRANT"];
    let variants = [
        KeyInputs { format: 2, ..base },
        KeyInputs {
            engine: "2.3.6",
            ..base
        },
        KeyInputs {
            executable: "another engine digest",
            ..base
        },
        KeyInputs {
            target: "x86_64-unknown-linux-gnu",
            ..base
        },
        KeyInputs { schema: 8, ..base },
        KeyInputs {
            usage_schema: 5,
            ..base
        },
        KeyInputs {
            definitions: &edited_definition,
            ..base
        },
        KeyInputs {
            definitions: &moved_definition,
            ..base
        },
        KeyInputs {
            statements: &edited_statement,
            ..base
        },
        KeyInputs {
            statements: &extra_statement,
            ..base
        },
    ];
    let mut keys = vec![compose(&base)];
    keys.extend(variants.iter().map(compose));
    assert_eq!(compose(&base), keys[0], "the key is not deterministic");
    let mut unique = keys.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), keys.len(), "an input did not change the key");
    // Length framing: moving a byte between adjacent fields changes the key.
    let split = ["ab", "c"];
    let joined = ["a", "bc"];
    assert_ne!(
        compose(&KeyInputs {
            statements: &split,
            ..base
        }),
        compose(&KeyInputs {
            statements: &joined,
            ..base
        })
    );
    // The compiled key: a valid template key, computed once from constants,
    // with no supervisor executable or source text among its inputs.
    let key = compiled_key();
    assert!(crate::server::valid_template_key(key), "{key}");
    assert_eq!(key.len(), 64);
    assert!(std::ptr::eq(key, compiled_key()));
    assert_eq!(crate::server::compiled_template_key(), key);
    assert_eq!(
        key,
        compose(&KeyInputs {
            format: TEMPLATE_FORMAT,
            engine: provision::DOLT_VERSION,
            executable: crate::catalog::BUNDLED_ASSET.executable_sha256,
            target: crate::catalog::BUNDLED_ASSET.target,
            schema: migrations::CURRENT_VERSION,
            usage_schema: migrations::USAGE_CURRENT_VERSION,
            definitions: &migrations::template_key_definitions(),
            statements: &creation_statements(),
        })
    );
    let [main, usage] = migrations::template_key_definitions();
    assert_eq!(main.len(), 6, "every main definition is keyed");
    assert_eq!(usage.len(), 3, "every usage definition is keyed");
}

/// Every statement that writes on a store's creation path comes from a
/// keyed constant: the bootstrap, initialization and migration-step bodies
/// hold no inline writing SQL, so adding one without keying it fails here.
#[test]
fn creation_statements_are_keyed() {
    const WRITES: [&str; 8] = [
        "\"CREATE ",
        "\"INSERT ",
        "\"UPDATE ",
        "\"DELETE ",
        "\"ALTER ",
        "\"GRANT ",
        "\"DROP ",
        "\"CALL DOLT_",
    ];
    /// The text of `function`'s body in `source`, up to the next item.
    fn body<'a>(source: &'a str, function: &str) -> &'a str {
        let start = source
            .find(function)
            .unwrap_or_else(|| panic!("{function} not found"));
        let rest = &source[start + function.len()..];
        let end = rest.find("\nfn ").unwrap_or(rest.len()).min(
            rest.find("\nasync fn ")
                .unwrap_or(rest.len())
                .min(rest.find("\npub").unwrap_or(rest.len())),
        );
        &rest[..end]
    }
    let server = include_str!("../../server.rs");
    let store = include_str!("../../store.rs");
    let migrations = include_str!("../migrations.rs");
    for (file, source, function) in [
        ("server.rs", server, "async fn initialize_database("),
        ("server.rs", server, "fn server_yaml("),
        ("store.rs", store, "async fn initialize(pool"),
        (
            "migrations.rs",
            migrations,
            "async fn discover_current_attempt_in(",
        ),
        ("migrations.rs", migrations, "async fn build_attempt("),
        ("migrations.rs", migrations, "async fn publish("),
        (
            "migrations.rs",
            migrations,
            "async fn ensure_usage_branch_at_v4(",
        ),
    ] {
        let text = body(source, function);
        for write in WRITES {
            assert!(
                !text.contains(write),
                "{file} {function} holds inline writing SQL starting {write}: key it as a constant \
                 in the store template key's statements"
            );
        }
    }
    // The keyed constants are the ones the creation path uses.
    let statements = creation_statements();
    for constant in [
        crate::server::BOOTSTRAP_CREATE_DATABASE,
        crate::server::BOOTSTRAP_CREATE_IDENTITY,
        crate::server::BOOTSTRAP_INSERT_IDENTITY,
        crate::server::BOOTSTRAP_READER_GRANT,
        crate::server::SERVER_BEHAVIOR,
        INITIALIZE_COMMIT,
        AUTHOR,
    ] {
        assert!(statements.contains(&constant), "{constant} is not keyed");
    }
    for statement in INITIALIZE_STATEMENTS
        .iter()
        .chain(migrations::TEMPLATE_KEY_STATEMENTS.iter())
    {
        assert!(statements.contains(statement), "{statement} is not keyed");
    }
    assert!(
        server.contains("{SERVER_BEHAVIOR}listener:"),
        "server.yaml no longer takes its behavior block from the keyed constant"
    );
}

/// A build publishes once: the capture holds `data/` only, synced before
/// its manifest, with none of the build store's path, secrets or host name;
/// the build store is gone; and a second call finds the template without a
/// build.
#[tokio::test]
async fn template_builds_once_with_data_only_and_is_reused_without_build() -> Result<()> {
    let fixture = fixture()?;
    let root = fixture.path().join("templates");
    let engine = engine().await?;
    let events = Arc::new(StdMutex::new(Vec::new()));
    let hooks = Hooks {
        events: Some(events.clone()),
        ..Hooks::default()
    };
    let started = Instant::now();
    let built = HOOKS
        .scope(hooks, ensure(&root, &engine))
        .await
        .map_err(failure)?;
    let elapsed = started.elapsed();
    let Ensured::Built(report) = built else {
        bail!("the first call did not build: {built:?}");
    };
    println!(
        "template build: {elapsed:?}, {} files, {} bytes, host name {:?}",
        report.files, report.bytes, report.hostname
    );
    ensure!(report.published, "the build did not publish");
    assert_eq!(
        entries_of(&root)?,
        [key().to_owned(), lock_name(key())],
        "the root holds more than the template and its lock"
    );
    ensure!(
        !report.build_store.exists(),
        "the build store outlived the capture"
    );
    let template = root.join(key());
    let manifest = read_manifest(&template)?;
    assert_eq!(entries_of(&template)?, [DATA, MANIFEST]);
    assert_eq!(manifest.key, key());
    assert_eq!(manifest.format, TEMPLATE_FORMAT);
    assert_eq!(
        report.files,
        manifest
            .entries
            .iter()
            .filter(|entry| matches!(entry, Entry::File { .. }))
            .count()
    );
    // T12: no build-private bytes, by the build's own needles.
    ensure!(!report.needles.is_empty() && !report.hostname.is_empty());
    let hits = scan(&template.join(DATA), &report.needles).map_err(failure)?;
    ensure!(hits.is_empty(), "the published template leaks: {hits:?}");
    // T11: every captured file and directory was synced before the
    // manifest was written.
    let events = events.lock().expect("hook events").clone();
    let manifest_at = events
        .iter()
        .position(|event| *event == Event::Manifest)
        .context("no manifest event")?;
    let synced: BTreeSet<String> = events[..manifest_at]
        .iter()
        .filter_map(|event| match event {
            Event::Synced(path) => {
                let text = path.to_string_lossy().replace('\\', "/");
                text.split("/data/").nth(1).map(str::to_owned)
            }
            Event::Manifest => None,
        })
        .collect();
    for entry in &manifest.entries {
        ensure!(
            synced.contains(&entry.path()),
            "{} was not synced before the manifest",
            entry.path()
        );
    }
    // T3: the second call reuses it and creates no build.
    let identity = published(&root).context("no published template")?;
    let reused = ensure(&root, &engine).await.map_err(failure)?;
    ensure!(matches!(reused, Ensured::Published), "{reused:?}");
    assert_eq!(entries_of(&root)?, [key().to_owned(), lock_name(key())]);
    assert_eq!(published(&root), Some(identity));
    fixture.release(Ok(()))
}

/// The byte scan's positive control: every spelling of every needle the
/// capture derives (both encodings, both separators, a Windows verbatim
/// prefix stripped, each secret whole and in fragments) is found in a file
/// that holds it, and a clean file yields nothing. Host names shorter than
/// the scanned minimum produce no needle.
#[test]
fn byte_scan_finds_every_spelling_of_every_needle() -> Result<()> {
    let fixture = fixture()?;
    let secrets = [
        "root-secret-0123456789abcdefghijklmnopqrstuv".to_owned(),
        "reader-secret-ZYXWVUTSRQPONMLKJIHGFEDCBA98".to_owned(),
    ];
    let paths = [
        PathBuf::from(r"\\?\C:\Users\builder\kuru\.build-key\store"),
        PathBuf::from("/private/var/kuru/.build-key/store"),
    ];
    let short = "abc";
    assert!(short.len() < HOSTNAME_NEEDLE_MIN);
    let needles = leak_needles(
        &paths,
        &secrets,
        &["build-host-01".to_owned(), short.to_owned()],
    );
    let labels: BTreeSet<&str> = needles.iter().map(|needle| needle.label.as_str()).collect();
    for expected in [
        "the build store path (UTF-8)",
        "the build store path (UTF-16LE)",
        "the root secret (UTF-8)",
        "the reader secret (UTF-16LE)",
        "a 16-character fragment of the root secret (UTF-8)",
        "a 16-character fragment of the reader secret (UTF-16LE)",
        "the host name (UTF-8)",
        "the host name (UTF-16LE)",
    ] {
        ensure!(
            labels.contains(expected),
            "no needle {expected}: {labels:?}"
        );
    }
    let spelled = |text: &str| -> Vec<Vec<u8>> {
        vec![
            text.as_bytes().to_vec(),
            text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        ]
    };
    let bytes: BTreeSet<Vec<u8>> = needles.iter().map(|needle| needle.bytes.clone()).collect();
    for form in [
        r"C:\Users\builder\kuru\.build-key\store",
        "C:/Users/builder/kuru/.build-key/store",
        "/private/var/kuru/.build-key/store",
        r"\private\var\kuru\.build-key\store",
        "build-host-01",
        secrets[0].as_str(),
        &secrets[1][..16],
    ] {
        for spelling in spelled(form) {
            ensure!(
                bytes.contains(&spelling),
                "{form:?} is not a needle in every encoding"
            );
        }
    }
    ensure!(
        !bytes.iter().any(|needle| needle.is_empty()),
        "an empty needle would match nothing"
    );
    for spelling in spelled(short) {
        ensure!(
            !bytes.contains(&spelling),
            "a short host name became a needle"
        );
    }
    // One file per needle, each holding only that needle between filler
    // bytes, and one clean file.
    let tree = files::ensure_private_directory(&fixture.path().join("tree"))?;
    let nested = tree.create_private_directory(OsStr::new("nested"))?;
    for (index, needle) in needles.iter().enumerate() {
        let parent = if index % 2 == 0 { &tree } else { &nested };
        let mut file = parent.create_new(OsStr::new(&format!("n{index:03}")))?;
        file.write_all(b"\x00filler before\xff")?;
        file.write_all(&needle.bytes)?;
        file.write_all(b"\xfefiller after\x00")?;
        file.sync_all()?;
    }
    let mut clean = tree.create_new(OsStr::new("clean"))?;
    clean.write_all(b"schema, receipts and the placeholder row")?;
    clean.sync_all()?;
    drop(clean);
    let hits = scan(tree.path(), &needles).map_err(failure)?;
    for (index, needle) in needles.iter().enumerate() {
        let file = if index % 2 == 0 {
            format!("n{index:03}")
        } else {
            format!("nested/n{index:03}")
        };
        ensure!(
            hits.contains(&(file.clone(), needle.label.clone())),
            "{} in {file} was not found: {hits:?}",
            needle.label
        );
    }
    ensure!(
        !hits.iter().any(|(file, _)| file == "clean"),
        "a clean file was reported: {hits:?}"
    );
    fixture.release(Ok(()))
}

/// A build whose captured bytes hold the build store's path is refused as
/// an engine failure (never a verdict), publishes nothing and removes its
/// capture stage at once; the key lock is free afterwards and the next
/// exclusive holder sweeps the build store and publishes a clean build.
#[tokio::test]
async fn byte_scan_hit_refuses_publication_and_discards_the_stage() -> Result<()> {
    let fixture = fixture()?;
    let root = fixture.path().join("templates");
    let engine = engine().await?;
    let hooks = Hooks {
        plant_leak: true,
        ..Hooks::default()
    };
    let refused = HOOKS
        .scope(hooks, ensure(&root, &engine))
        .await
        .expect_err("a build holding its own path was published");
    let text = format!("{refused}");
    ensure!(matches!(refused, CreationFailure::Engine(_)), "{text}");
    ensure!(
        text.contains("the build store path") && text.contains(hooks::PLANTED_LEAK),
        "{text}"
    );
    ensure!(
        published(&root).is_none(),
        "the leaking build was published"
    );
    let names = entries_of(&root)?;
    let builds: Vec<&String> = names
        .iter()
        .filter(|name| name.starts_with(&format!(".build-{}-", key())))
        .collect();
    ensure!(
        !names
            .iter()
            .any(|name| name.starts_with(&format!(".stage-{}-", key()))),
        "the leaking capture stage stayed in the cache: {names:?}"
    );
    let [build] = builds.as_slice() else {
        bail!("not exactly one build store left for a sweep: {names:?}");
    };
    let build = (*build).clone();
    release(hold(&root, Mode::Exclusive).await?).await;
    let rebuilt = ensure(&root, &engine).await.map_err(failure)?;
    let Ensured::Built(report) = rebuilt else {
        bail!("no rebuild: {rebuilt:?}");
    };
    ensure!(report.published, "the clean build did not publish");
    assert_eq!(report.swept.removed, [build]);
    assert_eq!(entries_of(&root)?, [key().to_owned(), lock_name(key())]);
    fixture.release(Ok(()))
}

/// Without waiting, a structural verdict quarantines the judged template
/// and is returned: nothing is built in the same call. Warm-up (waiting)
/// quarantines and then rebuilds.
#[tokio::test]
async fn structural_verdict_without_waiting_quarantines_and_never_rebuilds() -> Result<()> {
    let fixture = fixture()?;
    let root = fixture.path().join("templates");
    let template = root.join(key());
    let judged = clone_shared(&root).await?;
    let mut manifest = read_manifest(&template)?;
    manifest.format += 1;
    write_manifest(&template, &manifest)?;
    // A build store left beside the published template (its removal failed
    // after publication) and an abandoned capture stage of this key, and
    // another key's build store.
    let abandoned = plant_abandoned(&root, key())?;
    let other = plant_abandoned(&root, &"0".repeat(64))?;
    let refused = {
        // No engine starts; the sweep takes the abandoned build's lease.
        let _gate = crate::spawn_gate::locking_async().await;
        ensure_in(&root, &no_engine(), Wait::Never).await
    }
    .expect_err("a damaged template was accepted");
    ensure!(refused.is_verdict(), "{refused}");
    ensure!(published(&root).is_none(), "not quarantined");
    let quarantined = rejected(&root)?;
    let [only] = quarantined.as_slice() else {
        bail!("not exactly one quarantined directory: {quarantined:?}");
    };
    assert_eq!(files::directory(&root.join(only))?.identity(), judged);
    let names = entries_of(&root)?;
    // The quarantining holder sweeps this key's abandoned entries, as a
    // build would, and builds nothing.
    for gone in &abandoned {
        ensure!(!names.contains(gone), "{gone} was not swept: {names:?}");
    }
    for kept in &other {
        ensure!(names.contains(kept), "{kept} was removed: {names:?}");
    }
    ensure!(
        !names
            .iter()
            .any(|name| name.starts_with(&format!(".build-{}-", key()))),
        "a build started after the verdict: {names:?}"
    );
    // A warm-up quarantines and rebuilds.
    let identity = clone_shared(&root).await?;
    files::write(&template.join("extra"), b"extra")?;
    let engine = engine().await?;
    let warmed = {
        let _gate = crate::spawn_gate::spawning().await;
        let deadline = Instant::now() + crate::test_support::template_warm_up_bound();
        ensure_in(&root, &engine, Wait::Until(deadline)).await
    }
    .map_err(failure)?;
    ensure!(matches!(warmed, Ensured::Built(_)), "{warmed:?}");
    let rebuilt = published(&root).context("the warm-up did not publish")?;
    ensure!(
        rebuilt != identity,
        "the damaged template is still published"
    );
    let quarantined = rejected(&root)?;
    let [only] = quarantined.as_slice() else {
        bail!("not exactly one quarantined directory: {quarantined:?}");
    };
    assert_eq!(files::directory(&root.join(only))?.identity(), identity);
    fixture.release(Ok(()))
}

/// Two projects created at once with no template (design 3.9, build side):
/// while the first is inside its build engine, the second is told at once
/// that no template is available, copies nothing and reads no unpublished
/// name; once the first publishes, a later creator copies without a build.
#[tokio::test]
async fn second_creator_goes_cold_at_once_while_a_build_is_in_progress() -> Result<()> {
    let fixture = fixture()?;
    let root = fixture.path().join("templates");
    let engine = engine().await?;
    let pause = Arc::new(Pause::default());
    let hooks = Hooks {
        pause: Some(pause.clone()),
        ..Hooks::default()
    };
    let first = tokio::spawn({
        let (root, engine) = (root.clone(), engine.clone());
        HOOKS.scope(hooks, async move { ensure(&root, &engine).await })
    });
    let bound = startup() + QUERY_TIMEOUT * 4;
    tokio::time::timeout(bound, pause.reached.notified())
        .await
        .context("the first build did not reach its pause")?;
    // The first creator is inside its build engine: a build store exists
    // and nothing is published.
    let names = entries_of(&root)?;
    ensure!(
        names
            .iter()
            .any(|name| name.starts_with(&format!(".build-{}-", key())))
            && published(&root).is_none(),
        "the first creator is not mid-build: {names:?}"
    );
    // The second creator takes no lock gate: the first holds the spawn
    // gate through its pause, and the second never acquires a lock.
    let second = stage(&fixture, "second")?;
    let started = Instant::now();
    let created = create_in(&root, &no_engine(), &second)
        .await
        .map_err(failure)?;
    let ensured = ensure_in(&root, &no_engine(), Wait::Never)
        .await
        .map_err(failure)?;
    let waited = started.elapsed();
    ensure!(waited < PROMPT, "the second creator waited {waited:?}");
    ensure!(
        matches!(created, Created::Unavailable(Unavailable::Busy)),
        "{created:?}"
    );
    ensure!(
        matches!(ensured, Ensured::Unavailable(Unavailable::Busy)),
        "{ensured:?}"
    );
    ensure!(
        fs::read_dir(second.path())?.next().is_none(),
        "the second creator copied while the first was building"
    );
    println!("second creator answered in {waited:?} while the first was building");
    pause.resume.notify_one();
    let built = tokio::time::timeout(bound, first)
        .await
        .context("the first build did not finish after its pause")??
        .map_err(failure)?;
    let Ensured::Built(report) = built else {
        bail!("the first creator did not build: {built:?}");
    };
    ensure!(report.published);
    let third = stage(&fixture, "third")?;
    let copied = create_in(&root, &no_engine(), &third)
        .await
        .map_err(failure)?;
    ensure!(
        matches!(
            copied,
            Created::Copied {
                built: false,
                published: true
            }
        ),
        "{copied:?}"
    );
    fixture.release(Ok(()))
}

/// A held key lock never makes a template caller wait: without waiting it
/// is "no template now", and nothing unpublished is read or created.
#[tokio::test]
async fn concurrent_template_lock_acquisition_never_waits() -> Result<()> {
    let fixture = fixture()?;
    let root = fixture.path().join("templates");
    // A build in progress: the exclusive lock is held.
    let building = hold(&root, Mode::Exclusive).await?;
    let destination = stage(&fixture, "copy")?;
    let started = Instant::now();
    let ensured = ensure_in(&root, &no_engine(), Wait::Never)
        .await
        .map_err(failure)?;
    let created = create_in(&root, &no_engine(), &destination)
        .await
        .map_err(failure)?;
    ensure!(started.elapsed() < PROMPT, "a busy lock was waited for");
    ensure!(
        matches!(ensured, Ensured::Unavailable(Unavailable::Busy)),
        "{ensured:?}"
    );
    ensure!(
        matches!(created, Created::Unavailable(Unavailable::Busy)),
        "{created:?}"
    );
    release(building).await;
    // A copier holds the shared lock and nothing is published: a builder
    // cannot take the exclusive lock, and does not wait for it.
    let copying = hold(&root, Mode::Shared).await?;
    let ensured = ensure_in(&root, &no_engine(), Wait::Never)
        .await
        .map_err(failure)?;
    ensure!(
        matches!(ensured, Ensured::Unavailable(Unavailable::Busy)),
        "{ensured:?}"
    );
    release(copying).await;
    assert_eq!(entries_of(&root)?, [lock_name(key())]);
    ensure!(
        fs::read_dir(destination.path())?.next().is_none(),
        "a busy lock left a copy"
    );
    fixture.release(Ok(()))
}

/// A lock-file failure (opening, `TryLockError::Error`, identity) is "no
/// template" with a warning for a product caller, and fatal and named for a
/// warm-up; it quarantines nothing.
#[tokio::test]
async fn template_lock_errors_mean_no_template_and_are_fatal_in_warm_up() -> Result<()> {
    let fixture = fixture()?;
    let root = fixture.path().join("templates");
    let identity = clone_shared(&root).await?;
    for step in [LockStep::Open, LockStep::TryLock, LockStep::Verify] {
        let hooks = Hooks {
            lock: Some(step),
            ..Hooks::default()
        };
        let ensured = HOOKS
            .scope(hooks.clone(), ensure_in(&root, &no_engine(), Wait::Never))
            .await
            .map_err(failure)?;
        ensure!(
            matches!(&ensured, Ensured::Unavailable(Unavailable::Lock(text)) if text.contains("injected")),
            "{step:?}: {ensured:?}"
        );
        let destination = stage(&fixture, &format!("copy-{step:?}"))?;
        let created = HOOKS
            .scope(hooks.clone(), create_in(&root, &no_engine(), &destination))
            .await
            .map_err(failure)?;
        ensure!(
            matches!(&created, Created::Unavailable(Unavailable::Lock(_))),
            "{step:?}: {created:?}"
        );
        let deadline = Instant::now() + PROMPT;
        let warm = HOOKS
            .scope(hooks, ensure_in(&root, &no_engine(), Wait::Until(deadline)))
            .await;
        let Err(error) = warm else {
            bail!("{step:?}: a lock error was not fatal in warm-up");
        };
        ensure!(
            !error.is_verdict() && error.to_string().contains("warm-up"),
            "{step:?}: {error}"
        );
        assert_eq!(published(&root), Some(identity), "{step:?}");
        ensure!(rejected(&root)?.is_empty(), "{step:?} quarantined");
    }
    // An unusable root is the same: here it is a file.
    let file_root = fixture.path().join("not-a-root");
    files::write(&file_root, b"not a directory")?;
    let ensured = ensure_in(&file_root, &no_engine(), Wait::Never)
        .await
        .map_err(failure)?;
    ensure!(
        matches!(ensured, Ensured::Unavailable(Unavailable::Lock(_))),
        "{ensured:?}"
    );
    fixture.release(Ok(()))
}

/// The exclusive holder sweeps this key's abandoned build store (after one
/// quiescence attempt) and capture stage, leaves a build store whose
/// lifecycle lease is still held without waiting for it, never touches
/// another key's entries, and builds in its own directory.
#[tokio::test]
async fn half_built_template_is_swept_after_quiescence_and_rebuilt() -> Result<()> {
    let fixture = fixture()?;
    let root = fixture.path().join("templates");
    let engine = engine().await?;
    let directory = open_root(&root)?;
    let lifecycle_root = build_lifecycle_root(&directory)?;
    let abandoned_build = |name: &str| -> Result<PathBuf> {
        let build = directory.create_private_directory(OsStr::new(name))?;
        let store = build.create_private_directory(OsStr::new(BUILD_STORE))?;
        store.create_private_directory(OsStr::new(DATA))?;
        Ok(store.path().to_owned())
    };
    let dead = format!(".build-{}-{}", key(), Uuid::new_v4());
    abandoned_build(&dead)?;
    let live = format!(".build-{}-{}", key(), Uuid::new_v4());
    let live_store = abandoned_build(&live)?;
    let capture = format!(".stage-{}-{}", key(), Uuid::new_v4());
    let partial = directory.create_private_directory(OsStr::new(&capture))?;
    files::write(&partial.path().join("partial"), b"a partial capture")?;
    // An abandoned stage has no live handle; a held one would keep its name
    // delete-pending on Windows.
    drop(partial);
    let other = "0".repeat(64);
    let other_build = format!(".build-{other}-{}", Uuid::new_v4());
    abandoned_build(&other_build)?;
    let other_stage = format!(".stage-{other}-{}", Uuid::new_v4());
    directory.create_private_directory(OsStr::new(&other_stage))?;
    // T26: a builder that is still alive holds its build store's lease.
    let lease = {
        let _gate = crate::spawn_gate::locking_async().await;
        Server::quiescence_at(&live_store, lifecycle_root.as_deref(), PROMPT).await?
    };
    let started = Instant::now();
    let built = ensure(&root, &engine).await.map_err(failure)?;
    let elapsed = started.elapsed();
    let Ensured::Built(report) = built else {
        bail!("no build: {built:?}");
    };
    {
        let _gate = crate::spawn_gate::locking_async().await;
        drop(lease);
    }
    let mut removed = report.swept.removed.clone();
    removed.sort();
    let mut expected = vec![dead.clone(), capture.clone()];
    expected.sort();
    assert_eq!(removed, expected, "{:?}", report.swept);
    let left: Vec<&str> = report
        .swept
        .left
        .iter()
        .map(|left| left.name.as_str())
        .collect();
    assert_eq!(left, [live.as_str()], "{:?}", report.swept);
    // The held lease is why it was left, and the error says so.
    ensure!(!report.swept.left[0].error.is_empty(), "{:?}", report.swept);
    ensure!(report.published);
    let names = entries_of(&root)?;
    for kept in [&live, &other_build, &other_stage] {
        ensure!(names.contains(kept), "{kept} was removed: {names:?}");
    }
    for gone in [&dead, &capture] {
        ensure!(!names.contains(gone), "{gone} was not swept: {names:?}");
    }
    println!("build beside a held abandoned lease: {elapsed:?}");
    // The live builder's store has a lease the fixture guard reads.
    crate::test_support::await_store_quiescence(&live_store, lifecycle_root.as_deref()).await?;
    fixture.release(Ok(()))
}

/// A publication that fails leaves the verified capture stage in place:
/// the build's caller copies from it, so the chain is paid once, and the
/// next exclusive holder sweeps it and publishes a fresh build.
#[tokio::test]
async fn publication_failure_copies_from_verified_stage() -> Result<()> {
    let fixture = fixture()?;
    let root = fixture.path().join("templates");
    let engine = engine().await?;
    let destination = stage(&fixture, "copy")?;
    let hooks = Hooks {
        refuse_publication: true,
        ..Hooks::default()
    };
    let created = HOOKS
        .scope(hooks, create(&root, &engine, &destination))
        .await
        .map_err(failure)?;
    ensure!(
        matches!(
            created,
            Created::Copied {
                built: true,
                published: false
            }
        ),
        "{created:?}"
    );
    ensure!(
        published(&root).is_none(),
        "the refused stage was published"
    );
    let stages: Vec<String> = entries_of(&root)?
        .into_iter()
        .filter(|name| name.starts_with(&format!(".stage-{}-", key())))
        .collect();
    let [left] = stages.as_slice() else {
        bail!("not exactly one verified stage left: {stages:?}");
    };
    verify(&files::directory(&root.join(left))?, key()).map_err(failure)?;
    let copied = destination.path().join(DATA);
    ensure!(copied.is_dir(), "nothing was copied from the stage");
    let manifest = read_manifest(&root.join(left))?;
    let found = walk(&copied, Walk::Template, None, false, &mut |_, _| Ok(())).map_err(failure)?;
    assert_eq!(found, manifest.entries, "the copy differs from the stage");
    let rebuilt = ensure(&root, &engine).await.map_err(failure)?;
    let Ensured::Built(report) = rebuilt else {
        bail!("no rebuild: {rebuilt:?}");
    };
    assert_eq!(report.swept.removed, std::slice::from_ref(left));
    ensure!(report.published && published(&root).is_some());
    fixture.release(Ok(()))
}

/// A structural verdict quarantines the judged template before any copy; a
/// manifest read error is not a verdict and leaves it published. At most one
/// quarantined directory per key is kept.
#[tokio::test]
async fn template_failing_structure_is_quarantined() -> Result<()> {
    let fixture = fixture()?;
    let root = fixture.path().join("templates");
    let template = root.join(key());
    // A manifest read error: not a verdict, the template stays.
    let identity = clone_shared(&root).await?;
    let destination = stage(&fixture, "read-error")?;
    let hooks = Hooks {
        manifest_read_error: true,
        ..Hooks::default()
    };
    let error = HOOKS
        .scope(hooks, create_in(&root, &no_engine(), &destination))
        .await
        .expect_err("a manifest read error was accepted");
    ensure!(matches!(error, CreationFailure::Io(_)), "{error}");
    assert_eq!(published(&root), Some(identity));
    ensure!(rejected(&root)?.is_empty());
    // Verdicts: each quarantines the template it judged, and no copy starts.
    type Damage<'a> = Box<dyn Fn() -> Result<()> + 'a>;
    let mut cases: Vec<(&str, Damage<'_>)> = vec![
        (
            "another key",
            Box::new(|| {
                let mut manifest = read_manifest(&template)?;
                manifest.key = "f".repeat(64);
                write_manifest(&template, &manifest)
            }),
        ),
        (
            "another format",
            Box::new(|| {
                let mut manifest = read_manifest(&template)?;
                manifest.format += 1;
                write_manifest(&template, &manifest)
            }),
        ),
        (
            "an extra top-level entry",
            Box::new(|| files::write(&template.join("extra"), b"extra")),
        ),
        (
            "an unparsable manifest",
            Box::new(|| files::write(&template.join(MANIFEST), b"{")),
        ),
    ];
    for (case, damage) in cases.drain(..) {
        if published(&root).is_none() {
            clone_shared(&root).await?;
        }
        let judged = published(&root).context("no template")?;
        damage()?;
        let destination = stage(&fixture, &format!("copy-{}", case.replace(' ', "-")))?;
        let error = create_in(&root, &no_engine(), &destination)
            .await
            .expect_err("a damaged template was accepted");
        ensure!(error.is_verdict(), "{case}: {error}");
        ensure!(published(&root).is_none(), "{case}: not quarantined");
        let quarantined = rejected(&root)?;
        let [only] = quarantined.as_slice() else {
            bail!("{case}: not exactly one quarantined directory: {quarantined:?}");
        };
        assert_eq!(
            files::directory(&root.join(only))?.identity(),
            judged,
            "{case}: another directory was quarantined"
        );
        ensure!(
            fs::read_dir(destination.path())?.next().is_none(),
            "{case}: a copy started"
        );
    }
    fixture.release(Ok(()))
}

/// A verdict while copying (a digest or size other than the manifest's, an
/// extra hard link, a link) quarantines the template and leaves the partial
/// copy; a read error while copying leaves both the copy and the template.
#[tokio::test]
async fn template_byte_corruption_mid_copy_preserves_remnant_and_quarantines() -> Result<()> {
    let fixture = fixture()?;
    let root = fixture.path().join("templates");
    // An I/O error mid-copy: no verdict, the template stays.
    let identity = clone_shared(&root).await?;
    let largest = largest_file(&root.join(key()))?;
    let destination = stage(&fixture, "read-error")?;
    let fault = ReadFault::new(&largest.file_name().context("no name")?.to_string_lossy());
    let hooks = Hooks {
        read: Some(fault.clone()),
        ..Hooks::default()
    };
    let error = HOOKS
        .scope(hooks, create_in(&root, &no_engine(), &destination))
        .await
        .expect_err("an injected read error was accepted");
    ensure!(fault.fired(), "the read fault never fired");
    ensure!(matches!(error, CreationFailure::Io(_)), "{error}");
    assert_eq!(published(&root), Some(identity));
    ensure!(rejected(&root)?.is_empty());
    ensure!(
        destination.path().join(DATA).is_dir(),
        "the partial copy was not left for preservation"
    );
    // Each damage takes the template's largest file and a directory outside
    // the template.
    type Damage = Box<dyn Fn(&Path, &Path) -> Result<()>>;
    let mut cases: Vec<(&str, Damage)> = vec![
        (
            "a changed byte",
            Box::new(|file, _| {
                let mut bytes = fs::read(file)?;
                let middle = bytes.len() / 2;
                bytes[middle] ^= 1;
                Ok(fs::write(file, bytes)?)
            }),
        ),
        (
            "an extra byte",
            Box::new(|file, _| {
                let mut handle = fs::OpenOptions::new().append(true).open(file)?;
                Ok(handle.write_all(b"x")?)
            }),
        ),
    ];
    #[cfg(unix)]
    cases.push((
        "an extra hard link",
        Box::new(|file, outside| Ok(fs::hard_link(file, outside.join("alias"))?)),
    ));
    #[cfg(unix)]
    cases.push((
        "a symbolic link",
        Box::new(|file, _| {
            let link = file.with_file_name("zz-link");
            Ok(std::os::unix::fs::symlink(file, link)?)
        }),
    ));
    for (case, damage) in cases.drain(..) {
        if published(&root).is_none() {
            clone_shared(&root).await?;
        }
        let judged = published(&root).context("no template")?;
        let outside = fixture
            .path()
            .join(format!("outside-{}", case.replace(' ', "-")));
        files::private_dir(&outside)?;
        damage(&largest_file(&root.join(key()))?, &outside)?;
        let destination = stage(&fixture, &format!("copy-{}", case.replace(' ', "-")))?;
        let error = create_in(&root, &no_engine(), &destination)
            .await
            .expect_err("a damaged template was copied");
        ensure!(error.is_verdict(), "{case}: {error}");
        // The outside link goes before a later quarantine removes this one.
        let _ = fs::remove_file(outside.join("alias"));
        let quarantined = rejected(&root)?;
        let [only] = quarantined.as_slice() else {
            bail!("{case}: not exactly one quarantined directory: {quarantined:?}");
        };
        assert_eq!(files::directory(&root.join(only))?.identity(), judged);
        ensure!(published(&root).is_none(), "{case}: not quarantined");
        ensure!(
            destination.path().join(DATA).is_dir(),
            "{case}: the partial copy was not left"
        );
    }
    fixture.release(Ok(()))
}

/// A quarantine moves only the directory that was judged: a template
/// republished between the verdict and the exclusive lock is left in place.
#[tokio::test]
async fn quarantine_is_bound_to_the_judged_template() -> Result<()> {
    let fixture = fixture()?;
    let root = fixture.path().join("templates");
    let template = root.join(key());
    // A fresh, valid template another process will publish in the window.
    let fresh_root = fixture.path().join("fresh");
    let fresh = clone_shared(&fresh_root).await?;
    clone_shared(&root).await?;
    let mut manifest = read_manifest(&template)?;
    manifest.key = "f".repeat(64);
    write_manifest(&template, &manifest)?;
    let (from, aside, replacement) = (
        template.clone(),
        root.join("judged-aside"),
        fresh_root.join(key()),
    );
    let hooks = Hooks {
        before_quarantine: Some(Arc::new(move || {
            fs::rename(&from, &aside).expect("move the judged template aside");
            fs::rename(&replacement, &from).expect("publish a fresh template");
        })),
        ..Hooks::default()
    };
    let destination = stage(&fixture, "copy")?;
    let error = HOOKS
        .scope(hooks, create_in(&root, &no_engine(), &destination))
        .await
        .expect_err("a template with another key was accepted");
    ensure!(error.is_verdict(), "{error}");
    assert_eq!(
        published(&root),
        Some(fresh),
        "the fresh template was moved"
    );
    ensure!(rejected(&root)?.is_empty(), "something was quarantined");
    let Inspection::Valid(_) = inspect(&open_root(&root)?, key()).map_err(failure)? else {
        bail!("the fresh template is not valid");
    };
    fixture.release(Ok(()))
}

/// Every failure that is not a completed comparison against the bytes is
/// engine-side or I/O, never a verdict.
#[test]
fn only_a_template_verdict_is_a_verdict() {
    let verdict = anyhow::Error::from(TemplateVerdict::new("placeholder differs"))
        .context("open and adopt the copied template stage");
    assert!(CreationFailure::classify(verdict).is_verdict());
    for error in [
        anyhow!("Dolt exited before readiness (exit status: 1)"),
        anyhow!("Dolt database bootstrap deadline exceeded while adopting the store template"),
        anyhow::Error::from(std::io::Error::other("lost reply")),
        anyhow!("memory store template key \"a\" differs from this supervisor's compiled key"),
        anyhow!("authenticated Dolt startup deadline exceeded"),
    ] {
        let text = format!("{error:#}");
        let classified = CreationFailure::classify(error);
        assert!(
            matches!(classified, CreationFailure::Engine(_)),
            "{text} was classified as {classified}"
        );
    }
    assert!(!CreationFailure::io(std::io::Error::other("EIO")).is_verdict());
}

/// No call removes another key's template, quarantined directory, build or
/// capture stage, or any lock file: a build and a copy of this key leave
/// every other entry byte-identical.
#[tokio::test]
async fn open_never_removes_other_keys_or_lock_files() -> Result<()> {
    let fixture = fixture()?;
    let root = fixture.path().join("templates");
    let engine = engine().await?;
    let directory = open_root(&root)?;
    let (first, second) = ("1".repeat(64), "2".repeat(64));
    for other in [&first, &second] {
        let template = directory.create_private_directory(OsStr::new(other))?;
        let data = template.create_private_directory(OsStr::new(DATA))?;
        files::write(&data.path().join("file"), other.as_bytes())?;
        files::write(&template.path().join(MANIFEST), b"{}")?;
        files::write(&root.join(lock_name(other)), b"")?;
    }
    let quarantined = directory
        .create_private_directory(OsStr::new(&format!(".rejected-{first}-{}", Uuid::new_v4())))?;
    files::write(&quarantined.path().join("file"), b"rejected")?;
    directory
        .create_private_directory(OsStr::new(&format!(".build-{second}-{}", Uuid::new_v4())))?;
    directory
        .create_private_directory(OsStr::new(&format!(".stage-{second}-{}", Uuid::new_v4())))?;
    let others = |names: &[String]| -> Vec<String> {
        names
            .iter()
            .filter(|name| !name.contains(key()))
            .cloned()
            .collect()
    };
    let before = snapshot(&root, &others(&entries_of(&root)?))?;
    let built = ensure(&root, &engine).await.map_err(failure)?;
    ensure!(matches!(built, Ensured::Built(_)), "{built:?}");
    let destination = stage(&fixture, "copy")?;
    let copied = create(&root, &engine, &destination)
        .await
        .map_err(failure)?;
    ensure!(
        matches!(copied, Created::Copied { built: false, .. }),
        "{copied:?}"
    );
    let after = snapshot(&root, &others(&entries_of(&root)?))?;
    assert_eq!(before, after, "another key's entries changed");
    for other in [&first, &second] {
        ensure!(
            root.join(lock_name(other)).is_file(),
            "a lock file was removed"
        );
    }
    fixture.release(Ok(()))
}

/// Every object beneath `names` of `root`: kind, size, digest and
/// modification time.
fn snapshot(root: &Path, names: &[String]) -> Result<BTreeMap<String, String>> {
    fn visit(path: &Path, label: String, into: &mut BTreeMap<String, String>) -> Result<()> {
        let metadata = fs::symlink_metadata(path)?;
        let modified = metadata.modified().ok();
        if metadata.is_dir() {
            into.insert(label.clone(), format!("directory {modified:?}"));
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                visit(
                    &entry.path(),
                    format!("{label}/{}", entry.file_name().to_string_lossy()),
                    into,
                )?;
            }
        } else {
            into.insert(
                label,
                format!(
                    "file {} {} {modified:?}",
                    metadata.len(),
                    hex(&Sha256::digest(fs::read(path)?))
                ),
            );
        }
        Ok(())
    }
    let mut into = BTreeMap::new();
    for name in names {
        visit(&root.join(name), name.clone(), &mut into)?;
    }
    Ok(into)
}

/// The build's own shape check refuses a template with a non-table object,
/// so nothing is published; and on the pinned engine every non-table object
/// query answers, zero when the object is absent and nonzero once it exists.
#[tokio::test]
async fn non_table_objects_fail_the_template_shape() -> Result<()> {
    let fixture = fixture()?;
    let root = fixture.path().join("templates");
    let engine = engine().await?;
    let hooks = Hooks {
        before_shape: vec!["CREATE VIEW kuru_template_probe AS SELECT 1 AS probe".into()],
        ..Hooks::default()
    };
    let refused = HOOKS.scope(hooks, ensure(&root, &engine)).await;
    let Err(error) = refused else {
        bail!("a build with a view was published");
    };
    ensure!(error.is_verdict(), "{error}");
    println!("build with a view: {error}");
    ensure!(
        published(&root).is_none(),
        "the refused build was published"
    );
    // S11 on a served copy of the shared template, under a build identity.
    let store = fixture.path().join("served").join("store");
    let parent = files::ensure_private_directory(store.parent().context("no parent")?)?;
    let served = parent.create_private_directory(OsStr::new("store"))?;
    crate::test_support::warm_runtime_cache().await?;
    let shared = open_root(&crate::test_support::shared_template_root()?)?;
    {
        let lock = {
            let _gate = crate::spawn_gate::locking_async().await;
            try_key_lock(&shared, key(), Mode::Shared)?.context("the shared template is locked")?
        };
        let Inspection::Valid(judged) = inspect(&shared, key()).map_err(failure)? else {
            bail!("no shared template");
        };
        copy_into(&judged, &served).map_err(failure)?;
        release(lock).await;
    }
    crate::server::write_template_build_identity(served.path(), key())?;
    let lifecycle_root = cfg!(windows).then(|| fixture.path().join("lifecycles"));
    let options = ServerOptions {
        binary: engine.binary.clone(),
        directory: served.path().to_owned(),
        project_scope: TEMPLATE_SCOPE.to_owned(),
        supervisor: engine.supervisor.clone(),
        timeout: engine.timeout,
        read_only: false,
        retained: None,
        lifecycle_root,
    };
    let server = {
        let _gate = crate::spawn_gate::spawning().await;
        Server::open(options).await?
    };
    let observed = async {
        let main = server.pool("main").await?;
        let mut connection = main.acquire().await?.detach();
        let mut report = Vec::new();
        let absent = migrations::template_shape::non_table_objects(&mut connection).await?;
        report.push(format!("none: {absent:?}"));
        ensure!(absent.iter().all(|(_, count)| *count == 0), "{absent:?}");
        for (object, create, drop, expected) in [
            (
                "a view",
                "CREATE VIEW kuru_probe_view AS SELECT 1 AS probe",
                "DROP VIEW kuru_probe_view",
                &["views", "stored schema objects"][..],
            ),
            (
                "a trigger",
                "CREATE TRIGGER kuru_probe_trigger BEFORE INSERT ON messages FOR EACH ROW SET NEW.role = NEW.role",
                "DROP TRIGGER kuru_probe_trigger",
                &["triggers", "stored schema objects"][..],
            ),
            (
                "a procedure",
                "CREATE PROCEDURE kuru_probe_procedure() SELECT 1",
                "DROP PROCEDURE kuru_probe_procedure",
                &["routines", "stored procedures"][..],
            ),
            (
                "an ignore rule",
                "INSERT INTO dolt_ignore VALUES ('kuru_probe_%', true)",
                "DELETE FROM dolt_ignore WHERE pattern = 'kuru_probe_%'",
                &["ignore rules"][..],
            ),
        ] {
            sqlx::Executor::execute(&mut connection, create).await?;
            let counts = migrations::template_shape::non_table_objects(&mut connection).await?;
            report.push(format!("{object}: {counts:?}"));
            let reported: Vec<&str> = counts
                .iter()
                .filter(|(_, count)| *count != 0)
                .map(|(name, _)| *name)
                .collect();
            ensure!(
                expected.iter().any(|name| reported.contains(name)),
                "{object} was not reported: {counts:?}"
            );
            sqlx::Executor::execute(&mut connection, drop).await?;
        }
        connection.close().await?;
        Ok::<_, anyhow::Error>(report)
    }
    .await;
    let closed = server.close().await;
    let report = observed?;
    closed?;
    println!(
        "S11 non-table objects on the pinned engine:\n{}",
        report.join("\n")
    );
    fixture.release(Ok(()))
}

/// A cancelled build keeps the key lock, its engine's reap guard, until
/// the engine is reaped.
#[tokio::test]
async fn cancelled_build_releases_key_lock_only_after_reap() -> Result<()> {
    let fixture = fixture()?;
    let root = fixture.path().join("templates");
    let canonical = {
        files::ensure_private_directory(&root)?;
        fs::canonicalize(&root)?
    };
    let engine = engine().await?;
    let pause = Arc::new(Pause::default());
    let hooks = Hooks {
        pause: Some(pause.clone()),
        ..Hooks::default()
    };
    let building = tokio::spawn({
        let root = root.clone();
        HOOKS.scope(
            hooks,
            async move { ensure(&root, &engine).await.map(|_| ()) },
        )
    });
    let deadline = startup() + QUERY_TIMEOUT * 4;
    tokio::time::timeout(deadline, pause.reached.notified())
        .await
        .context("the build did not reach its pause")?;
    building.abort();
    ensure!(
        building.await.is_err_and(|error| error.is_cancelled()),
        "the build finished instead of being cancelled"
    );
    let started = Instant::now();
    let live_at_acquisition = loop {
        let acquired = {
            let directory = open_root(&root)?;
            let _gate = crate::spawn_gate::locking_async().await;
            try_key_lock(&directory, key(), Mode::Exclusive)?.map(|lock| {
                let live = engine_ledger::with(|ledger| ledger.live_under(&canonical));
                drop(lock);
                live
            })
        };
        if let Some(live) = acquired {
            break live;
        }
        ensure!(
            started.elapsed() < deadline,
            "the key lock stayed held {deadline:?} after the cancelled build"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    ensure!(
        live_at_acquisition.is_empty(),
        "the key lock was released before the build engine was reaped: {live_at_acquisition:?}"
    );
    fixture.release(Ok(()))
}

/// A failed template warm-up is not cached: the next warm-up tries again and
/// builds, and only its success is kept.
#[tokio::test]
async fn failed_template_warm_up_is_not_cached() -> Result<()> {
    let fixture = fixture()?;
    let root = fixture.path().join("templates");
    let engine = engine().await?;
    let cell = tokio::sync::OnceCell::new();
    let bound = crate::test_support::template_warm_up_bound();
    let failing = Hooks {
        lock: Some(LockStep::Verify),
        ..Hooks::default()
    };
    let error = HOOKS
        .scope(
            failing.clone(),
            crate::test_support::warm_template_in(&cell, &root, &engine, bound),
        )
        .await
        .expect_err("an injected lock failure warmed");
    ensure!(
        format!("{error:#}").contains("warm the test store template cache"),
        "{error:#}"
    );
    ensure!(cell.get().is_none(), "a failure was cached");
    let warmed = crate::test_support::warm_template_in(&cell, &root, &engine, bound).await?;
    assert_eq!(warmed, crate::test_support::TemplateWarmUp::Built);
    // A success is cached: the same fault no longer reaches the template.
    let again = HOOKS
        .scope(
            failing,
            crate::test_support::warm_template_in(&cell, &root, &engine, bound),
        )
        .await?;
    assert_eq!(again, crate::test_support::TemplateWarmUp::Built);
    fixture.release(Ok(()))
}

/// The synchronous warm-up refuses to block a Tokio runtime and names the
/// async form, without starting its thread.
#[tokio::test]
async fn warm_blocking_refuses_inside_a_runtime() {
    let threads = crate::test_support::WARM_THREADS.load(std::sync::atomic::Ordering::Relaxed);
    let error = crate::test_support::warm_blocking().expect_err("blocked inside a runtime");
    assert!(
        error.to_string().contains("warmed_cache_dir"),
        "the refusal does not name the async form: {error}"
    );
    let refused = crate::test_support::cache_dir().expect_err("cache_dir blocked a runtime");
    assert!(refused.to_string().contains("warmed_cache_dir"));
    assert_eq!(
        crate::test_support::WARM_THREADS.load(std::sync::atomic::Ordering::Relaxed),
        threads,
        "a warm-up thread was started"
    );
}

/// A writable fresh open of fixture options that were never warmed fails
/// at once, before any engine start; warmed options, reopens, read-only
/// opens, private template roots and product options are never refused.
#[tokio::test]
async fn unwarmed_fixture_open_fails_the_guard_before_any_start() -> Result<()> {
    crate::test_support::warm_runtime_cache().await?;
    let root = crate::test_support::tempdir()?;
    let canonical = fs::canonicalize(root.path())?;
    let scope = format!("project/{}", "9".repeat(64));
    let data = root.path().join("private");
    let unwarmed = crate::test_support::open_options(data.clone(), scope.clone())?;
    let error = crate::test_support::spawn_gated_open(unwarmed.clone())
        .await
        .expect_err("an unwarmed fixture created its store");
    ensure!(
        format!("{error:#}").contains(crate::store::UNWARMED_FIXTURE),
        "{error:#}"
    );
    ensure!(
        engine_ledger::with(|ledger| ledger.live_under(&canonical)).is_empty(),
        "an engine started"
    );
    let memory = data.join("memory");
    let stores: Vec<String> = fs::read_dir(&memory)?
        .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .filter(|name| name != "locks" && name != "lifecycles")
        .collect();
    ensure!(stores.is_empty(), "the refused open left {stores:?}");
    // The predicate alone: only unwarmed, writable, shared-cache options.
    let mut cases = Vec::new();
    let mut warmed = unwarmed.clone();
    warmed.fixture = Some(Fixture::Warmed);
    cases.push(("warmed", warmed));
    let mut read_only = unwarmed.clone();
    read_only.read_only = true;
    cases.push(("read-only", read_only));
    let mut private = unwarmed.clone();
    private.template_root = Some(root.path().join("templates"));
    cases.push(("private template root", private));
    let mut product = unwarmed.clone();
    product.fixture = None;
    cases.push(("product", product));
    let mut elsewhere = unwarmed.clone();
    elsewhere.config.cache_dir = Some(root.path().join("cache"));
    cases.push(("another cache", elsewhere));
    for (case, options) in cases {
        ensure!(
            options.refuse_unwarmed_fixture().is_ok(),
            "{case} was refused"
        );
    }
    ensure!(unwarmed.refuse_unwarmed_fixture().is_err());
    // A reopen of an existing store is never guarded.
    let created = crate::test_support::spawn_gated_open(unwarmed.clone().warmed().await?).await?;
    created.close().await?;
    let reopened = crate::test_support::spawn_gated_open(unwarmed).await?;
    reopened.close().await?;
    root.release(Ok(()))
}

/// The teardown guard fails a fixture whose open built or quarantined the
/// shared store template, naming the event, unless the fixture opted in;
/// only the shared root's events are charged.
#[test]
fn fixture_open_that_builds_a_template_fails_the_guard() -> Result<()> {
    let guarded = crate::test_support::tempdir()?;
    let stage = guarded.path().join("memory").join("stage");
    files::private_dir(&stage)?;
    engine_ledger::template_event(&stage, engine_ledger::TemplateEvent::Built);
    let verdict = guarded
        .release(Ok(()))
        .expect_err("a fixture that built the shared template passed teardown");
    let text = format!("{verdict:#}");
    ensure!(
        text.contains("built the shared store template")
            && text.contains("allowing_template_build"),
        "{text}"
    );
    let allowed = crate::test_support::tempdir()?.allowing_template_build();
    let stage = allowed.path().join("stage");
    files::private_dir(&stage)?;
    engine_ledger::template_event(&stage, engine_ledger::TemplateEvent::Quarantined);
    allowed.release(Ok(()))?;
    // Only the shared test root is charged.
    let private = crate::test_support::tempdir()?;
    ensure!(!crate::test_support::is_shared_template_root(
        private.path()
    ));
    let shared = crate::test_support::shared_template_root();
    if let Ok(shared) = shared
        && shared.exists()
    {
        ensure!(crate::test_support::is_shared_template_root(&shared));
    }
    private.release(Ok(()))
}

const CHILD_OUTCOME: &str = "KURU_TEST_TEMPLATE_CACHE_CHILD_OUTCOME";
const CHILD_TEST: &str = "store::creation_template::tests::child_process_warms_its_template_cache";
const CHILD_DEADLINE: Duration = Duration::from_secs(600);

/// Runs only as a child of
/// `warm_runtime_cache_builds_one_template_across_processes`: the first
/// fixture of a process, `MemoryStore::temporary()`, warms the cache its
/// environment names before anything else, and its open builds nothing.
#[tokio::test]
async fn child_process_warms_its_template_cache() -> Result<()> {
    let Some(outcome) = std::env::var_os(CHILD_OUTCOME) else {
        return Ok(());
    };
    let store = MemoryStore::temporary().await?;
    let warmed = crate::test_support::warmed_template();
    let root = crate::test_support::shared_template_root()?;
    let published = matches!(
        inspect(&open_root(&root)?, key()).map_err(failure)?,
        Inspection::Valid(_)
    );
    store.close().await?;
    let charged = engine_ledger::with(|ledger| ledger.template_events_under(Path::new("/")));
    files::write(
        Path::new(&outcome),
        format!("{warmed:?} published={published} charged={}", charged.len()).as_bytes(),
    )
}

#[cfg(unix)]
async fn spawn_child(cache: &Path, outcome: &Path, log: &Path) -> Result<tokio::process::Child> {
    let mut command = tokio::process::Command::new(std::env::current_exe()?);
    command
        .args(["--exact", CHILD_TEST, "--nocapture", "--test-threads=1"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env(CHILD_OUTCOME, outcome)
        .env("KURU_DOLT_CACHE", cache)
        .env("TMPDIR", std::env::temp_dir())
        .stdin(std::process::Stdio::null())
        .stdout(File::create(log)?)
        .stderr(File::create(log.with_extension("stderr"))?)
        .kill_on_drop(true);
    for name in ["KURU_TEST_SUPERVISOR_PREPARED", "LLVM_PROFILE_FILE"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    for (name, value) in crate::test_support::lifecycle_trace::forwarded() {
        command.env(name, value);
    }
    let _gate = crate::spawn_gate::spawning().await;
    Ok(command.spawn()?)
}

#[cfg(unix)]
async fn wait_child(mut child: tokio::process::Child) -> Result<std::process::ExitStatus> {
    tokio::time::timeout(CHILD_DEADLINE, child.wait())
        .await
        .context("template cache child process exceeded its deadline")?
        .map_err(Into::into)
}

#[cfg(windows)]
async fn spawn_child(
    cache: &Path,
    outcome: &Path,
    log: &Path,
) -> Result<kuru_platform::windows::process::NativeChild> {
    use kuru_platform::windows::process::{Console, NativeSpawnSpec, Stdio as NativeStdio};
    let system = kuru_platform::windows::process::system_directory()?;
    let windows = system
        .parent()
        .context("Windows system directory has no parent")?;
    let mut command = NativeSpawnSpec::new(
        std::env::current_exe()?,
        cache.parent().context("cache has no parent")?.to_owned(),
    );
    command.args = ["--exact", CHILD_TEST, "--nocapture", "--test-threads=1"]
        .into_iter()
        .map(Into::into)
        .collect();
    command.environment = vec![
        ("SystemRoot".into(), windows.as_os_str().into()),
        ("PATH".into(), system.into_os_string()),
        (CHILD_OUTCOME.into(), outcome.as_os_str().into()),
        ("KURU_DOLT_CACHE".into(), cache.as_os_str().into()),
    ];
    for name in [
        "KURU_TEST_SUPERVISOR_PREPARED",
        "LLVM_PROFILE_FILE",
        "TMP",
        "TEMP",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.environment.push((name.into(), value));
        }
    }
    command
        .environment
        .extend(crate::test_support::lifecycle_trace::forwarded());
    let stdout: std::os::windows::io::OwnedHandle = File::create(log)?.into();
    let stderr: std::os::windows::io::OwnedHandle =
        File::create(log.with_extension("stderr"))?.into();
    command.stdin = NativeStdio::Null;
    command.stdout = NativeStdio::Handle(stdout);
    command.stderr = NativeStdio::Handle(stderr);
    command.console = Console::PrivateHidden;
    let _gate = crate::spawn_gate::spawning().await;
    Ok(command.spawn().await?)
}

#[cfg(windows)]
async fn wait_child(
    mut child: kuru_platform::windows::process::NativeChild,
) -> Result<std::process::ExitStatus> {
    Ok(child.wait(CHILD_DEADLINE).await?)
}

/// Two test processes sharing a fresh cache warm it concurrently: exactly
/// one builds the store template, the other waits for it and finds it
/// published, and each first fixture (`MemoryStore::temporary()`) warmed
/// before it opened and built nothing in its open.
#[tokio::test]
async fn warm_runtime_cache_builds_one_template_across_processes() -> Result<()> {
    let fixture = fixture()?;
    let cache = fixture.path().join("cache");
    files::private_dir(&cache)?;
    let mut children = Vec::new();
    for name in ["first", "second"] {
        let outcome = fixture.path().join(format!("{name}-outcome"));
        let log = fixture.path().join(format!("{name}.log"));
        children.push((spawn_child(&cache, &outcome, &log).await?, outcome, log));
    }
    let mut outcomes = Vec::new();
    for (child, outcome, log) in children {
        let status = wait_child(child).await?;
        let diagnostics = format!(
            "stdout: {}\nstderr: {}",
            fs::read_to_string(&log).unwrap_or_default(),
            fs::read_to_string(log.with_extension("stderr")).unwrap_or_default()
        );
        ensure!(status.success(), "child failed ({status}): {diagnostics}");
        outcomes.push(String::from_utf8(files::read_bytes(&outcome, 256)?)?);
    }
    outcomes.sort();
    assert_eq!(
        outcomes,
        [
            "Some(Built) published=true charged=0",
            "Some(Published) published=true charged=0",
        ],
        "not exactly one build across the processes"
    );
    let root = root_in(&fs::canonicalize(&cache)?);
    assert_eq!(entries_of(&root)?, [key().to_owned(), lock_name(key())]);
    verify(&files::directory(&root.join(key()))?, key()).map_err(failure)?;
    fixture.release(Ok(()))
}
