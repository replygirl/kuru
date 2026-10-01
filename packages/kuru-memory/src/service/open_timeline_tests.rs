//! The owner open timeline at close: written once, after the owner lock is
//! released, never failing the close, and only by a gated owner.

use super::tests::{attach_raw, owner_fixture, owner_lock_free};
use super::*;
use crate::open_timeline::{Event, Timeline, file_name, is_timeline_name};
use crate::test_support::{
    await_managed_quiescence, fixture_deadline, observed, warm_runtime_cache,
};

/// The timeline file names in `services`, sorted; none when it is absent.
fn timeline_names(services: &Path) -> Result<Vec<String>> {
    let entries = match std::fs::read_dir(services) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut names = Vec::new();
    for entry in entries {
        let name = entry?.file_name();
        if is_timeline_name(&name) {
            names.push(
                name.into_string()
                    .map_err(|name| anyhow::anyhow!("{name:?}"))?,
            );
        }
    }
    names.sort();
    Ok(names)
}

fn stamped_timeline() -> Arc<Timeline> {
    let timeline = Arc::new(Timeline::new());
    for event in Event::ALL {
        timeline.stamp(event);
    }
    timeline
}

// Test 6.
#[tokio::test]
async fn the_record_appears_only_after_the_owner_lock_is_released() -> Result<()> {
    warm_runtime_cache().await?;
    let deadline = fixture_deadline(1, 0);
    tokio::time::timeout(deadline, async {
        let root = tempfile::tempdir()?;
        let (project, scope, data, options) = owner_fixture(root.path())?;
        let services = EndpointRecord::directory(&data, &scope)?;
        let _gate = crate::spawn_gate::spawning().await;
        let owner = ServiceOwner::open(options.clone(), &project).await?;
        let generation = owner.authority().service_generation.clone();
        let points = [
            ClosePoint::BeforeListenerDrop,
            ClosePoint::AfterListenerDrop,
            ClosePoint::AfterEndpointRetire,
            ClosePoint::AfterReap,
        ];
        let pause = ClosePause::at_each(&points);
        let (mut knobs, _events) = observed(Admission::AnyAttachment, None);
        knobs.close_pause = Some(pause.clone());
        knobs.timeline = Some(stamped_timeline());
        let served = tokio::spawn(owner.serve_with(knobs));
        drop(attach_raw(&data, &scope, None).await?);
        for point in points {
            pause.entered.notified().await;
            ensure!(
                timeline_names(&services)?.is_empty(),
                "a timeline was written by {point:?}"
            );
            ensure!(
                !owner_lock_free(&options)?,
                "the owner lock was free at {point:?}"
            );
            pause.release.notify_one();
        }
        served.await??;
        ensure!(owner_lock_free(&options)?);
        let names = timeline_names(&services)?;
        ensure!(names == [file_name(&generation)], "{names:?}");
        let record: serde_json::Value =
            serde_json::from_slice(&std::fs::read(services.join(&names[0]))?)?;
        ensure!(record["service_generation"] == generation.as_str());
        ensure!(record["events"].as_array().map(Vec::len) == Some(Event::ALL.len()));
        await_managed_quiescence(&options).await
    })
    .await
    .with_context(|| format!("timeline ordering fixture exceeded its {deadline:?} deadline"))?
}

// Test 7.
#[tokio::test]
async fn a_failed_record_write_leaves_the_close_unchanged() -> Result<()> {
    warm_runtime_cache().await?;
    let deadline = fixture_deadline(1, 0);
    tokio::time::timeout(deadline, async {
        let root = tempfile::tempdir()?;
        let (project, scope, data, options) = owner_fixture(root.path())?;
        let services = EndpointRecord::directory(&data, &scope)?;
        let _gate = crate::spawn_gate::spawning().await;
        let owner = ServiceOwner::open(options.clone(), &project).await?;
        // A directory where the record would be created.
        let taken = services.join(file_name(&owner.authority().service_generation));
        std::fs::create_dir(&taken)?;
        let (mut knobs, _events) = observed(Admission::AnyAttachment, None);
        knobs.timeline = Some(stamped_timeline());
        let served = tokio::spawn(owner.serve_with(knobs));
        drop(attach_raw(&data, &scope, None).await?);
        served.await??;
        ensure!(EndpointRecord::read(&data, &scope)?.is_none());
        ensure!(owner_lock_free(&options)?);
        await_managed_quiescence(&options).await?;
        ensure!(taken.is_dir(), "the occupied name was replaced");
        ensure!(std::fs::read_dir(&taken)?.next().is_none());
        Ok::<(), anyhow::Error>(())
    })
    .await
    .with_context(|| format!("timeline failure fixture exceeded its {deadline:?} deadline"))?
}

/// Spawn one real owner (with `environment` added), keep its endpoint
/// record, then release its starter and await its process exit.
#[cfg(all(unix, feature = "test-support"))]
async fn run_child_owner(
    options: &crate::store::OpenOptions,
    project: &Path,
    log: &Path,
    environment: Vec<(OsString, OsString)>,
) -> Result<EndpointRecord> {
    let executable = options
        .supervisor
        .clone()
        .context("fixture supervisor absent")?;
    let diagnostic = File::create(log)?;
    let owner = activity::with_owner_environment(environment, async {
        let _gate = crate::spawn_gate::spawning().await;
        spawn_logged_owner_fixture(options, project, &executable, diagnostic).await
    })
    .await?;
    let record = EndpointRecord::read(&options.data_dir, &options.project_scope)?
        .context("the child owner published no endpoint")?;
    owner.exited(fixture_deadline(0, 1)).await?;
    Ok(record)
}

// Test 8.
#[cfg(all(unix, feature = "test-support"))]
#[tokio::test]
async fn a_gated_owner_writes_one_complete_record() -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    warm_runtime_cache().await?;
    let deadline = fixture_deadline(1, 1);
    tokio::time::timeout(deadline, async {
        let root = tempfile::tempdir()?;
        let (project, scope, data, options) = owner_fixture(root.path())?;
        let services = EndpointRecord::directory(&data, &scope)?;
        // Create the store ungated, so the gated run is an existing-store open.
        run_child_owner(
            &options,
            &project,
            &root.path().join("first.log"),
            Vec::new(),
        )
        .await?;
        ensure!(timeline_names(&services)?.is_empty());
        let gate = vec![(OsString::from("KURU_OPEN_TIMELINE"), OsString::from("1"))];
        let record =
            run_child_owner(&options, &project, &root.path().join("second.log"), gate).await?;
        let authority = &record.authority;
        let names = timeline_names(&services)?;
        ensure!(
            names == [file_name(&authority.service_generation)],
            "{names:?}"
        );
        let path = services.join(&names[0]);
        ensure!(std::fs::metadata(&path)?.permissions().mode() & 0o777 == 0o600);
        let bytes = std::fs::read(&path)?;
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        ensure!(value["format"] == "kuru.open-timeline", "{value}");
        ensure!(value["format_version"] == 1, "{value}");
        ensure!(
            value["kuru_version"] == env!("CARGO_PKG_VERSION"),
            "{value}"
        );
        ensure!(
            value["service_generation"] == authority.service_generation.as_str(),
            "{value}"
        );
        ensure!(value["anchor_unix_ns"].is_u64(), "{value}");
        ensure!(value["counts"]["usage_rows"].is_u64(), "{value}");
        ensure!(value["dropped"] == 0 && value["late"] == 0, "{value}");
        let events = value["events"].as_array().context("no events")?;
        let names = events
            .iter()
            .map(|entry| entry["event"].as_str().unwrap_or_default())
            .collect::<Vec<_>>();
        let canonical = Event::ALL.map(Event::name);
        ensure!(names == canonical, "{names:?}");
        let offsets = events
            .iter()
            .map(|entry| entry["ns"].as_u64().context("an offset is not an integer"))
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            offsets.windows(2).all(|pair| pair[0] <= pair[1]),
            "{offsets:?}"
        );
        let text = String::from_utf8(bytes)?;
        let store = crate::store::project_directory(&data, &scope)?;
        let hash = store
            .file_name()
            .and_then(OsStr::to_str)
            .context("store has no name")?;
        for (label, forbidden) in [
            ("data directory", data.to_string_lossy().into_owned()),
            ("project path", project.to_string_lossy().into_owned()),
            ("scope", scope.clone()),
            ("scope hash", hash.to_owned()),
            ("connection secret", authority.connection_secret.clone()),
            ("store instance", authority.store_instance.clone()),
        ] {
            ensure!(!forbidden.is_empty(), "{label} is empty");
            ensure!(!text.contains(&forbidden), "the record carries the {label}");
        }
        await_managed_quiescence(&options).await
    })
    .await
    .with_context(|| format!("gated owner fixture exceeded its {deadline:?} deadline"))?
}

// Test 9.
#[cfg(all(unix, feature = "test-support"))]
#[tokio::test]
async fn an_ungated_owner_writes_no_record() -> Result<()> {
    warm_runtime_cache().await?;
    let deadline = fixture_deadline(1, 1);
    tokio::time::timeout(deadline, async {
        let root = tempfile::tempdir()?;
        let (project, scope, data, options) = owner_fixture(root.path())?;
        let services = EndpointRecord::directory(&data, &scope)?;
        for run in ["first", "second"] {
            let log = root.path().join(format!("{run}.log"));
            run_child_owner(&options, &project, &log, Vec::new()).await?;
            let names = timeline_names(&services)?;
            ensure!(names.is_empty(), "{run} ungated owner wrote {names:?}");
        }
        await_managed_quiescence(&options).await
    })
    .await
    .with_context(|| format!("ungated owner fixture exceeded its {deadline:?} deadline"))?
}
