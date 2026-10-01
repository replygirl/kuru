//! The open-activity record: the open stages an owner's own observed open has
//! begun, published beside its endpoint so the client that started it can say
//! what Kuru is doing before that endpoint exists.
//!
//! The record grants nothing. Election, attachment, recovery and retirement
//! never read it, and a client presents only the record tagged from the
//! starter token it passed. The tag is derived from that token, never the
//! token itself, which admits the starter's attachment. Every write and read
//! is best effort: neither can fail, cancel or delay an open.

#[cfg(test)]
use std::ffi::OsString;
use std::{
    ffi::OsStr,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use kuru_platform::fs::{NameRetention, Privacy, Publication, PublicationPhase};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::watch;

use super::EndpointRecord;
use crate::progress::{MemoryOpenStage, ProgressReporter};
use crate::store::{MemoryStore, OpenOptions};

const RECORD: &str = "activity.json";
/// Private stage for a record leaving its name; never read as a record.
const RETIRED_RECORD: &str = "activity.retired";
const RECORD_LIMIT: u64 = 4 * 1024;
const FORMAT: u32 = 1;
/// Separates this tag from any other digest of the same token.
const TAG_CONTEXT: &[u8] = b"kuru-open-activity-v1\0";
/// After a failed write the publisher waits this long, or for a newer list,
/// then writes the latest list again. It never retries the open.
const REWRITE_AFTER: Duration = Duration::from_millis(100);

/// Test-support hook: `1` makes every write of an owner's record fail.
#[cfg(any(test, feature = "test-support"))]
pub const WRITE_FAILURE_ENV: &str = "KURU_TEST_MEMORY_ACTIVITY_WRITE_FAILURE";

/// Test-support hook: names a directory. When an owner's open begins stage
/// `X` and `<dir>/X.hold` exists, the owner waits until a record holding `X`
/// was written, then stops advancing its open until that file is removed.
#[cfg(any(test, feature = "test-support"))]
pub const OPEN_HOLD_DIR_ENV: &str = "KURU_TEST_MEMORY_OPEN_HOLD_DIR";

/// The record tag for one starter token: a keyed SHA-256 digest in lowercase
/// hex, so the record never carries the token that admits the starter.
pub(crate) fn activity_tag(token: &uuid::Uuid) -> String {
    Sha256::new()
        .chain_update(TAG_CONTEXT)
        .chain_update(token.as_bytes())
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn is_tag(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// The record's name for a stage an owner may publish. Ready, the
/// retained-install notices and the client's own service start are never
/// published. Exhaustive, so a new stage must be placed here to compile.
fn stage_name(stage: MemoryOpenStage) -> Option<&'static str> {
    Some(match stage {
        MemoryOpenStage::WaitingForProjectOwnership => "WaitingForProjectOwnership",
        MemoryOpenStage::WaitingForRuntimeCache => "WaitingForRuntimeCache",
        MemoryOpenStage::VerifyingRuntimeCache => "VerifyingRuntimeCache",
        MemoryOpenStage::ExtractingEmbeddedRuntime => "ExtractingEmbeddedRuntime",
        MemoryOpenStage::CheckingRuntimeVersion => "CheckingRuntimeVersion",
        MemoryOpenStage::PreparingDatabase => "PreparingDatabase",
        MemoryOpenStage::OpeningDatabase => "OpeningDatabase",
        MemoryOpenStage::CreatingDatabase => "CreatingDatabase",
        MemoryOpenStage::UpgradingDatabase => "UpgradingDatabase",
        MemoryOpenStage::Ready
        | MemoryOpenStage::RetainedInstallStage
        | MemoryOpenStage::RetainedUnreceiptedInstallStage
        | MemoryOpenStage::StartingMemoryService => return None,
    })
}

fn named_stage(name: &str) -> Option<MemoryOpenStage> {
    MemoryOpenStage::ALL
        .into_iter()
        .find(|stage| stage_name(*stage) == Some(name))
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ActivityRecord {
    format: u32,
    tag: String,
    stages: Vec<String>,
}

fn encode(tag: &str, stages: &[MemoryOpenStage]) -> Result<Vec<u8>> {
    let stages = stages
        .iter()
        .map(|stage| {
            stage_name(*stage)
                .map(str::to_owned)
                .context("open stage is not published")
        })
        .collect::<Result<_>>()?;
    let bytes = serde_json::to_vec(&ActivityRecord {
        format: FORMAT,
        tag: tag.to_owned(),
        stages,
    })?;
    ensure!(
        bytes.len() as u64 <= RECORD_LIMIT,
        "open activity record exceeds its limit"
    );
    Ok(bytes)
}

/// A record's tag and stages. An unknown field, format, tag shape or stage
/// name makes the whole record unreadable.
fn decode(bytes: &[u8]) -> Result<(String, Vec<MemoryOpenStage>)> {
    let record: ActivityRecord =
        serde_json::from_slice(bytes).context("decode open activity record")?;
    ensure!(
        record.format == FORMAT,
        "unknown open activity record format"
    );
    ensure!(is_tag(&record.tag), "open activity record has no valid tag");
    ensure!(
        record.stages.len() <= MemoryOpenStage::ALL.len(),
        "open activity record lists too many stages"
    );
    let stages = record
        .stages
        .iter()
        .map(|name| named_stage(name).context("unknown open stage in activity record"))
        .collect::<Result<_>>()?;
    Ok((record.tag, stages))
}

/// The owner-private directory that also holds the endpoint record.
fn directory(data_dir: &Path, scope: &str) -> Result<PathBuf> {
    EndpointRecord::directory(data_dir, scope)
}

fn write_record(directory: &Path, tag: &str, stages: &[MemoryOpenStage]) -> Result<()> {
    let bytes = encode(tag, stages)?;
    crate::files::ensure_private_directory(directory)?;
    crate::files::write(&directory.join(RECORD), &bytes)
}

fn read_stages(data_dir: &Path, scope: &str, tag: &str) -> Result<Vec<MemoryOpenStage>> {
    let bytes = crate::files::read_bytes(&directory(data_dir, scope)?.join(RECORD), RECORD_LIMIT)?;
    let (found, stages) = decode(&bytes)?;
    ensure!(
        found == tag,
        "open activity record belongs to another owner"
    );
    Ok(stages)
}

/// Forward, in order, the stages of the record tagged `tag` that this client
/// has not yet forwarded. A missing, unreadable, foreign or concurrently
/// replaced record is ignored for this poll only, never logged or counted.
pub(crate) fn forward_new(
    data_dir: &Path,
    scope: &str,
    tag: &str,
    forwarded: &mut usize,
    progress: &mut ProgressReporter,
) {
    if !progress.is_observed() {
        return;
    }
    let Ok(stages) = read_stages(data_dir, scope, tag) else {
        return;
    };
    for stage in stages.iter().skip(*forwarded) {
        progress.report(*stage);
    }
    *forwarded = (*forwarded).max(stages.len());
}

/// How the publisher writes; inert in a release build.
#[derive(Clone, Default)]
pub(crate) struct Writes {
    #[cfg(any(test, feature = "test-support"))]
    fail: bool,
    /// Writes that fail before the publisher's own writes proceed.
    #[cfg(test)]
    failures: Option<Arc<std::sync::atomic::AtomicUsize>>,
    /// Each write first takes a permit; a closed gate fails it.
    #[cfg(test)]
    gate: Option<Arc<tokio::sync::Semaphore>>,
}

impl Writes {
    async fn write(
        &self,
        directory: &Path,
        tag: &str,
        stages: Arc<[MemoryOpenStage]>,
    ) -> Result<()> {
        #[cfg(test)]
        let _permit = match &self.gate {
            Some(gate) => Some(gate.acquire().await?),
            None => None,
        };
        #[cfg(test)]
        if let Some(failures) = &self.failures
            && failures
                .fetch_update(
                    std::sync::atomic::Ordering::SeqCst,
                    std::sync::atomic::Ordering::SeqCst,
                    |left| left.checked_sub(1),
                )
                .is_ok()
        {
            anyhow::bail!("test open activity write failure");
        }
        #[cfg(any(test, feature = "test-support"))]
        ensure!(!self.fail, "test-support open activity write failure");
        let (directory, tag) = (directory.to_owned(), tag.to_owned());
        tokio::task::spawn_blocking(move || write_record(&directory, &tag, &stages)).await?
    }
}

/// Test hooks for one owner's record; empty in a release build.
#[derive(Default)]
pub(crate) struct OwnerHooks {
    writes: Writes,
    #[cfg(any(test, feature = "test-support"))]
    hold_dir: Option<PathBuf>,
    #[cfg(test)]
    hold: Option<(MemoryOpenStage, Arc<OpenHold>)>,
}

impl OwnerHooks {
    /// The owner process's own hooks. A release build reads no variable.
    pub(crate) fn from_env() -> Self {
        #[cfg(any(test, feature = "test-support"))]
        {
            Self {
                writes: Writes {
                    fail: std::env::var_os(WRITE_FAILURE_ENV).as_deref() == Some(OsStr::new("1")),
                    #[cfg(test)]
                    failures: None,
                    #[cfg(test)]
                    gate: None,
                },
                hold_dir: std::env::var_os(OPEN_HOLD_DIR_ENV)
                    .filter(|directory| !directory.is_empty())
                    .map(PathBuf::from),
                #[cfg(test)]
                hold: None,
            }
        }
        #[cfg(not(any(test, feature = "test-support")))]
        Self::default()
    }
}

/// An in-process test barrier at one published stage, modelled on
/// `ClosePause`: the owner notifies `entered` once a record holding the stage
/// was written, then waits for `release` before advancing its open.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct OpenHold {
    pub(crate) entered: tokio::sync::Notify,
    pub(crate) release: tokio::sync::Notify,
}

/// The open's side of a running publisher.
struct Feed {
    stages: Vec<MemoryOpenStage>,
    sender: watch::Sender<Arc<[MemoryOpenStage]>>,
    task: tokio::task::JoinHandle<()>,
    tag: String,
    /// The length of the last list written.
    #[cfg(any(test, feature = "test-support"))]
    written: watch::Receiver<usize>,
}

impl Feed {
    fn start(directory: PathBuf, tag: String, writes: Writes) -> Self {
        let (sender, stages) = watch::channel(Arc::<[MemoryOpenStage]>::from([]));
        let (written_sender, _written) = watch::channel(0);
        let task = tokio::spawn(publish(
            directory,
            tag.clone(),
            stages,
            writes,
            written_sender,
        ));
        Self {
            stages: Vec::new(),
            sender,
            task,
            tag,
            #[cfg(any(test, feature = "test-support"))]
            written: _written,
        }
    }

    /// Hand one stage to the publisher without waiting for any write, and
    /// say whether it was published.
    fn push(&mut self, stage: MemoryOpenStage) -> bool {
        if stage_name(stage).is_none() {
            return false;
        }
        self.stages.push(stage);
        self.sender.send_replace(Arc::from(self.stages.as_slice()));
        true
    }

    /// End the feed with its open. The publisher may finish the latest list,
    /// then stops; it never removes the record.
    fn finish(self) -> Publisher {
        drop(self.sender);
        Publisher {
            task: self.task,
            tag: self.tag,
            #[cfg(test)]
            written: self.written,
            #[cfg(test)]
            published: self.stages.len(),
        }
    }
}

/// Writes the latest cumulative list whenever it changes, on its own task, so
/// nothing in the open, endpoint publication or serving waits for a write.
async fn publish(
    directory: PathBuf,
    tag: String,
    mut stages: watch::Receiver<Arc<[MemoryOpenStage]>>,
    writes: Writes,
    written: watch::Sender<usize>,
) {
    // `changed` still yields a list sent just before the open ended.
    while stages.changed().await.is_ok() {
        loop {
            let latest = Arc::clone(&stages.borrow_and_update());
            let length = latest.len();
            if writes.write(&directory, &tag, latest).await.is_ok() {
                written.send_replace(length);
                break;
            }
            // The latest list holds every earlier one, so writing it again
            // repairs whatever a reader missed. Ends once the open has.
            tokio::select! {
                changed = stages.changed() => if changed.is_err() {
                    return;
                },
                () = tokio::time::sleep(REWRITE_AFTER) => if stages.has_changed().is_err() {
                    return;
                },
            }
        }
    }
}

/// The finished publisher of one owner's record, kept until the owner retires
/// that record within its own close or failed open.
pub(crate) struct Publisher {
    task: tokio::task::JoinHandle<()>,
    tag: String,
    #[cfg(test)]
    written: watch::Receiver<usize>,
    #[cfg(test)]
    published: usize,
}

#[cfg(test)]
impl Publisher {
    /// Wait until the publisher wrote every stage its open handed it.
    pub(crate) async fn settled(&self) -> Result<()> {
        let published = self.published;
        self.written
            .clone()
            .wait_for(|written| *written >= published)
            .await
            .map(|_| ())
            .context("the publisher stopped before writing every stage")
    }
}

/// Wait at a test hold on `stage`, after a record holding it was written.
#[cfg(any(test, feature = "test-support"))]
async fn hold(
    hooks: &OwnerHooks,
    stage: MemoryOpenStage,
    feed: &Feed,
    limit: Duration,
) -> Result<()> {
    let marker = hooks
        .hold_dir
        .as_ref()
        .map(|directory| directory.join(format!("{stage:?}.hold")))
        .filter(|marker| marker.exists());
    #[cfg(test)]
    let barrier = hooks
        .hold
        .as_ref()
        .filter(|(at, _)| *at == stage)
        .map(|(_, barrier)| Arc::clone(barrier));
    #[cfg(not(test))]
    let barrier: Option<()> = None;
    if marker.is_none() && barrier.is_none() {
        return Ok(());
    }
    let deadline = tokio::time::Instant::now() + limit;
    if !hooks.writes.fail {
        let published = feed.stages.len();
        tokio::time::timeout_at(
            deadline,
            feed.written
                .clone()
                .wait_for(|written| *written >= published),
        )
        .await
        .with_context(|| format!("open activity holding {stage:?} was not written in time"))?
        .map(|_| ())
        .with_context(|| format!("open activity holding {stage:?} was never written"))?;
    }
    #[cfg(test)]
    if let Some(barrier) = barrier {
        barrier.entered.notify_one();
        barrier.release.notified().await;
    }
    if let Some(marker) = marker {
        while marker.exists() {
            ensure!(
                tokio::time::Instant::now() < deadline,
                "open hold on {stage:?} was not released"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    Ok(())
}

/// Open the owner's store, publishing its stages when it was started with a
/// starter token. Nothing here waits for a write. On failure the record is
/// retired before returning, while the caller still holds owner authority.
pub(crate) async fn open_owner_store(
    options: OpenOptions,
    hooks: OwnerHooks,
) -> Result<(MemoryStore, Option<Publisher>)> {
    #[cfg(feature = "test-support")]
    let fixture_stages = super::fixture_startup_stages_enabled();
    #[cfg(not(feature = "test-support"))]
    let fixture_stages = false;
    // An owner from a starter that predates the token publishes nothing.
    let target = options.starter_token.as_ref().and_then(|token| {
        Some((
            directory(&options.data_dir, &options.project_scope).ok()?,
            activity_tag(token),
        ))
    });
    // Both opens are boxed: held inline together they doubled the owner
    // open's future, which then overflowed a 2 MiB Windows test thread.
    if target.is_none() && !fixture_stages {
        return Ok((Box::pin(MemoryStore::open(options)).await?, None));
    }
    let (data_dir, scope) = (options.data_dir.clone(), options.project_scope.clone());
    #[cfg(any(test, feature = "test-support"))]
    let hold_limit = Duration::from_secs(options.config.startup_timeout_secs);
    let mut feed = target.map(|(directory, tag)| Feed::start(directory, tag, hooks.writes.clone()));
    let (mut progress, opening) = MemoryStore::open_observed(options);
    let mut opening = Box::pin(opening);
    let mut progress_open = true;
    let result = loop {
        tokio::select! {
            biased;
            stage = progress.recv(), if progress_open => {
                let Some(stage) = stage else {
                    progress_open = false;
                    continue;
                };
                #[cfg(feature = "test-support")]
                if fixture_stages {
                    eprintln!("memory startup stage: {stage:?}");
                }
                if let Some(feed) = &mut feed
                    && feed.push(stage)
                {
                    // A failed hold is reported and the open proceeds; it is
                    // never cancelled here.
                    #[cfg(any(test, feature = "test-support"))]
                    if let Err(error) = hold(&hooks, stage, feed, hold_limit).await {
                        eprintln!("memory open hold failed: {error:#}");
                    }
                }
            }
            result = &mut opening => break result,
        }
    };
    let publisher = feed.map(Feed::finish);
    match result {
        Ok(store) => Ok((store, publisher)),
        Err(error) => {
            if let Some(publisher) = publisher {
                retire(publisher, &data_dir, &scope).await;
            }
            Err(error)
        }
    }
}

/// Retire this owner's record by moving it off its name and then removing it,
/// leaving a record with any other tag in place. Call only while holding the
/// owner lock, after the publisher's open has ended. Best effort: no failure
/// here can fail a close or change an open's outcome.
pub(crate) async fn retire(publisher: Publisher, data_dir: &Path, scope: &str) {
    let Publisher { task, tag, .. } = publisher;
    // The task ends with its open. Joining it first means no late write can
    // recreate the record after its retirement.
    let _ = task.await;
    let Ok(directory) = directory(data_dir, scope) else {
        return;
    };
    let _ = tokio::task::spawn_blocking(move || retire_record(&directory, &tag)).await;
}

fn retire_record(directory: &Path, tag: &str) -> Result<()> {
    let directory =
        crate::files::open_directory(directory, Privacy::OwnerOnly, NameRetention::Movable)?;
    let name = OsStr::new(RECORD);
    let mut file = directory.read(name)?;
    let mut bytes = Vec::new();
    (&mut file).take(RECORD_LIMIT + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= RECORD_LIMIT,
        "open activity record exceeds its limit"
    );
    directory.verify(name, &file)?;
    let (found, _) = decode(&bytes)?;
    ensure!(
        found == tag,
        "open activity record belongs to another owner"
    );
    // As for the endpoint: on Windows a deleted name stays occupied while a
    // reader holds it, so the record leaves its name before deletion.
    directory.rename_file(
        &directory,
        name,
        &file,
        OsStr::new(RETIRED_RECORD),
        Publication::ReplaceRegular,
    )?;
    match directory.remove_file(OsStr::new(RETIRED_RECORD), file) {
        // A reader still holds the staged record; Windows removes it when
        // that reader closes, and it is already off the record's name.
        Err(error) if error.phase == PublicationPhase::Uncertain => Ok(()),
        removed => removed.map_err(Into::into),
    }
}

/// An in-process open for a read-only inspection that found no owner,
/// forwarding its own stages, ready and retained-install notices included.
pub(crate) async fn open_local_forwarding(
    options: OpenOptions,
    reporter: &mut ProgressReporter,
) -> Result<MemoryStore> {
    let (mut progress, opening) = MemoryStore::open_observed(options);
    let mut opening = Box::pin(opening);
    let mut forward = |stage| reporter.report(stage);
    let mut progress_open = true;
    let result = loop {
        tokio::select! {
            biased;
            stage = progress.recv(), if progress_open => match stage {
                Some(stage) => forward(stage),
                None => progress_open = false,
            },
            result = &mut opening => break result,
        }
    };
    // Ready and any retained-install notice are sent just before the open
    // returns; dropping the open ends the stream after them.
    drop(opening);
    while let Some(stage) = progress.recv().await {
        forward(stage);
    }
    result
}

/// The Windows owner receives no inherited environment; forward the two
/// test-support hooks as the startup-stage diagnostic is forwarded.
#[cfg(all(windows, any(test, feature = "test-support")))]
pub(crate) fn forwarded_test_hooks() -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    [WRITE_FAILURE_ENV, OPEN_HOLD_DIR_ENV]
        .into_iter()
        .filter_map(|name| Some((name.into(), std::env::var_os(name)?)))
        .collect()
}

#[cfg(test)]
tokio::task_local! {
    /// Extra environment for the owners one test's own opens spawn, so a hook
    /// never reaches the runner's environment or another test's owner.
    static OWNER_TEST_ENVIRONMENT: Vec<(OsString, OsString)>;
}

#[cfg(test)]
pub(crate) fn owner_test_environment() -> Vec<(OsString, OsString)> {
    OWNER_TEST_ENVIRONMENT
        .try_with(Clone::clone)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryOpenProgress;
    use crate::service::tests::{attach_raw, owner_fixture, owner_lock_free};
    use crate::service::{
        Admission, ClosePause, ClosePoint, ServiceLock, ServiceLockKind, ServiceOwner,
    };
    use crate::test_support::{
        await_managed_quiescence, fixture_deadline, observed, warm_runtime_cache,
    };
    use anyhow::bail;
    use futures::FutureExt as _;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use uuid::Uuid;

    use MemoryOpenStage::{
        CheckingRuntimeVersion, CreatingDatabase, ExtractingEmbeddedRuntime, OpeningDatabase,
        PreparingDatabase, Ready, RetainedInstallStage, RetainedUnreceiptedInstallStage,
        StartingMemoryService, UpgradingDatabase, WaitingForProjectOwnership,
        WaitingForRuntimeCache,
    };

    const SAMPLE: [MemoryOpenStage; 6] = [
        WaitingForRuntimeCache,
        ExtractingEmbeddedRuntime,
        CheckingRuntimeVersion,
        PreparingDatabase,
        CreatingDatabase,
        OpeningDatabase,
    ];

    /// A private data directory and a well-formed scope, with no engine.
    fn record_fixture() -> Result<(tempfile::TempDir, PathBuf, String)> {
        let root = tempfile::tempdir()?;
        let data = root.path().join("private");
        Ok((root, data, format!("project/{}", "a".repeat(64))))
    }

    fn received(progress: &mut MemoryOpenProgress) -> Vec<MemoryOpenStage> {
        let mut stages = Vec::new();
        while let Some(Some(stage)) = progress.recv().now_or_never() {
            stages.push(stage);
        }
        stages
    }

    fn assert_ready_once_and_last(stages: &[MemoryOpenStage]) -> Result<()> {
        ensure!(
            stages.last() == Some(&Ready),
            "ready was not last: {stages:?}"
        );
        ensure!(
            stages.iter().filter(|stage| **stage == Ready).count() == 1,
            "ready was reported more than once: {stages:?}"
        );
        Ok(())
    }

    /// Poll the open while receiving its stages until `stage` arrives.
    async fn until_stage<F>(
        progress: &mut MemoryOpenProgress,
        opening: &mut std::pin::Pin<Box<F>>,
        stage: MemoryOpenStage,
        seen: &mut Vec<MemoryOpenStage>,
    ) -> Result<()>
    where
        F: Future<Output = Result<crate::MemoryStore>>,
    {
        loop {
            tokio::select! {
                biased;
                received = progress.recv() => match received {
                    Some(received) => {
                        seen.push(received);
                        if received == stage {
                            return Ok(());
                        }
                    }
                    None => bail!("the open's stages ended before {stage:?}: {seen:?}"),
                },
                result = opening.as_mut() => {
                    let outcome = result.map(|_| ());
                    bail!("the open finished before {stage:?}: {seen:?}; {outcome:?}");
                }
            }
        }
    }

    /// Finish the open, then collect every stage it reported.
    async fn finish<F>(
        mut progress: MemoryOpenProgress,
        mut opening: std::pin::Pin<Box<F>>,
        seen: &mut Vec<MemoryOpenStage>,
    ) -> Result<crate::MemoryStore>
    where
        F: Future<Output = Result<crate::MemoryStore>>,
    {
        let opened = opening.as_mut().await;
        drop(opening);
        while let Some(stage) = progress.recv().await {
            seen.push(stage);
        }
        opened
    }

    async fn with_owner_environment<F: Future>(
        environment: Vec<(OsString, OsString)>,
        future: F,
    ) -> F::Output {
        OWNER_TEST_ENVIRONMENT.scope(environment, future).await
    }

    #[test]
    fn the_tag_is_a_keyed_digest_and_never_the_token() {
        let token = Uuid::new_v4();
        let tag = activity_tag(&token);
        assert!(is_tag(&tag), "{tag}");
        assert_eq!(tag, activity_tag(&token));
        assert_ne!(tag, activity_tag(&Uuid::new_v4()));
        let unkeyed: String = Sha256::digest(token.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_ne!(tag, unkeyed);
        for raw in [token.to_string(), token.simple().to_string()] {
            assert!(!is_tag(&raw), "{raw}");
            assert!(!tag.contains(&raw));
        }
    }

    // Holding both store opens inline more than doubled the owner open's
    // future; the frames that build it then overflowed 2 MiB Windows test
    // threads. Building the futures runs nothing.
    #[test]
    fn the_owner_store_open_holds_no_store_open_inline() {
        let options = OpenOptions::new(
            PathBuf::from("unused"),
            format!("project/{}", "a".repeat(64)),
        );
        let owner = open_owner_store(options.clone(), OwnerHooks::default());
        let store = MemoryStore::open(options);
        let (owner, store) = (size_of_val(&owner), size_of_val(&store));
        assert!(
            owner < store / 2,
            "the owner's store open ({owner} bytes) holds a store open ({store} bytes) inline"
        );
    }

    // T9
    #[test]
    fn the_record_round_trips_only_published_stages() -> Result<()> {
        let tag = activity_tag(&Uuid::new_v4());
        let published: Vec<_> = MemoryOpenStage::ALL
            .into_iter()
            .filter(|stage| stage_name(*stage).is_some())
            .collect();
        for unpublished in [
            Ready,
            RetainedInstallStage,
            RetainedUnreceiptedInstallStage,
            StartingMemoryService,
        ] {
            ensure!(!published.contains(&unpublished), "{unpublished:?}");
            ensure!(encode(&tag, &[unpublished]).is_err(), "{unpublished:?}");
        }
        let (found, stages) = decode(&encode(&tag, &published)?)?;
        ensure!(found == tag && stages == published, "{stages:?}");
        Ok(())
    }

    // T9
    #[test]
    fn an_unknown_field_format_stage_or_tag_shape_makes_the_record_unreadable() -> Result<()> {
        let token = Uuid::new_v4();
        let tag = activity_tag(&token);
        let upper = tag.to_uppercase();
        ensure!(
            decode(
                format!(r#"{{"format":1,"tag":"{tag}","stages":["PreparingDatabase"]}}"#)
                    .as_bytes()
            )
            .is_ok()
        );
        for invalid in [
            format!(r#"{{"format":1,"tag":"{tag}","stages":[],"token":"{token}"}}"#),
            format!(r#"{{"format":1,"tag":"{tag}","stages":["Teleporting"]}}"#),
            format!(r#"{{"format":1,"tag":"{tag}","stages":["Ready"]}}"#),
            format!(r#"{{"format":2,"tag":"{tag}","stages":[]}}"#),
            format!(r#"{{"format":1,"tag":"{token}","stages":[]}}"#),
            format!(r#"{{"format":1,"tag":"{upper}","stages":[]}}"#),
            r#"{"format":1,"stages":[]}"#.to_owned(),
        ] {
            ensure!(decode(invalid.as_bytes()).is_err(), "{invalid}");
        }
        Ok(())
    }

    // T4, unit half; T9 wrong tag, raw token and size limit.
    #[test]
    fn a_client_presents_only_the_record_of_its_own_owner() -> Result<()> {
        let (_root, data, scope) = record_fixture()?;
        let directory = directory(&data, &scope)?;
        let own = activity_tag(&Uuid::new_v4());
        let (mut progress, mut reporter) = ProgressReporter::observed();
        let mut forwarded = 0;
        forward_new(&data, &scope, &own, &mut forwarded, &mut reporter);
        ensure!(forwarded == 0, "a missing record moved the client");

        // Another owner's record, written by the publisher's own code.
        let token = Uuid::new_v4();
        write_record(&directory, &activity_tag(&token), &[UpgradingDatabase])?;
        for start in [0, 1, 5] {
            forwarded = start;
            forward_new(&data, &scope, &own, &mut forwarded, &mut reporter);
            ensure!(forwarded == start, "a foreign record moved the client");
        }
        // A record carrying the raw token instead of its tag.
        crate::files::write(
            &directory.join(RECORD),
            format!(r#"{{"format":1,"tag":"{token}","stages":["UpgradingDatabase"]}}"#).as_bytes(),
        )?;
        forwarded = 0;
        for tag in [activity_tag(&token), token.to_string()] {
            forward_new(&data, &scope, &tag, &mut forwarded, &mut reporter);
        }
        ensure!(forwarded == 0, "a raw-token record moved the client");
        // A record of its own owner over the size limit.
        let mut oversized = encode(&own, &[UpgradingDatabase])?;
        oversized.resize(RECORD_LIMIT as usize + 1, b' ');
        crate::files::write(&directory.join(RECORD), &oversized)?;
        forward_new(&data, &scope, &own, &mut forwarded, &mut reporter);
        ensure!(forwarded == 0, "an oversized record moved the client");
        ensure!(received(&mut progress).is_empty());

        write_record(&directory, &own, &[PreparingDatabase])?;
        forward_new(&data, &scope, &own, &mut forwarded, &mut reporter);
        ensure!(forwarded == 1);
        ensure!(received(&mut progress) == [PreparingDatabase]);
        Ok(())
    }

    // The forwarded sequence does not depend on how many writes or reads
    // the owner's stages took.
    #[test]
    fn forwarding_does_not_depend_on_how_the_list_was_written_or_read() -> Result<()> {
        let (_root, data, scope) = record_fixture()?;
        let directory = directory(&data, &scope)?;
        let tag = activity_tag(&Uuid::new_v4());
        for reads in 0_u32..(1 << SAMPLE.len()) {
            let (mut progress, mut reporter) = ProgressReporter::observed();
            let mut forwarded = 0;
            for written in 1..=SAMPLE.len() {
                write_record(&directory, &tag, &SAMPLE[..written])?;
                if reads & (1 << (written - 1)) != 0 {
                    forward_new(&data, &scope, &tag, &mut forwarded, &mut reporter);
                    ensure!(forwarded == written, "reads {reads:#b}");
                }
            }
            forward_new(&data, &scope, &tag, &mut forwarded, &mut reporter);
            ensure!(forwarded == SAMPLE.len(), "reads {reads:#b}");
            ensure!(received(&mut progress) == SAMPLE, "reads {reads:#b}");
        }
        Ok(())
    }

    #[test]
    fn retirement_removes_only_this_owners_record() -> Result<()> {
        let (_root, data, scope) = record_fixture()?;
        let directory = directory(&data, &scope)?;
        let (own, foreign) = (activity_tag(&Uuid::new_v4()), activity_tag(&Uuid::new_v4()));
        write_record(&directory, &foreign, &[PreparingDatabase])?;
        ensure!(retire_record(&directory, &own).is_err());
        ensure!(read_stages(&data, &scope, &foreign)? == [PreparingDatabase]);
        write_record(&directory, &own, &[PreparingDatabase])?;
        retire_record(&directory, &own)?;
        ensure!(!directory.join(RECORD).exists(), "the own record remained");
        ensure!(
            !directory.join(RETIRED_RECORD).exists(),
            "the staged record remained"
        );
        Ok(())
    }

    // T8: native on every CI platform.
    #[test]
    fn a_held_record_is_replaced_and_its_retirement_frees_the_name_at_once() -> Result<()> {
        let (_root, data, scope) = record_fixture()?;
        let directory = directory(&data, &scope)?;
        let tag = activity_tag(&Uuid::new_v4());
        write_record(&directory, &tag, &[PreparingDatabase])?;
        let held = crate::files::read(&directory.join(RECORD), Privacy::OwnerOnly)?;
        write_record(&directory, &tag, &[PreparingDatabase, OpeningDatabase])?;
        ensure!(read_stages(&data, &scope, &tag)? == [PreparingDatabase, OpeningDatabase]);
        let replacement = crate::files::read(&directory.join(RECORD), Privacy::OwnerOnly)?;
        // Removal of the staged record may wait for these readers.
        retire_record(&directory, &tag)?;
        ensure!(
            read_stages(&data, &scope, &tag).is_err(),
            "the name kept the record"
        );
        write_record(&directory, &tag, &[CreatingDatabase])?;
        ensure!(read_stages(&data, &scope, &tag)? == [CreatingDatabase]);
        drop((held, replacement));
        Ok(())
    }

    #[tokio::test]
    async fn a_publisher_writes_the_cumulative_list_and_rewrites_after_a_failure() -> Result<()> {
        let (_root, data, scope) = record_fixture()?;
        let tag = activity_tag(&Uuid::new_v4());
        let failures = Arc::new(AtomicUsize::new(1));
        let writes = Writes {
            failures: Some(Arc::clone(&failures)),
            ..Writes::default()
        };
        let mut feed = Feed::start(directory(&data, &scope)?, tag.clone(), writes);
        for unpublished in [Ready, StartingMemoryService] {
            ensure!(!feed.push(unpublished), "{unpublished:?} was published");
        }
        let mut written = feed.written.clone();
        tokio::time::timeout(Duration::from_secs(10), async {
            // The owner reports a wait only while the startup lock is busy,
            // so the wait is published like any other stage it begins.
            ensure!(feed.push(WaitingForProjectOwnership));
            // The first write fails; with no newer list it is written again.
            written.wait_for(|written| *written == 1).await?;
            ensure!(failures.load(Ordering::SeqCst) == 0);
            ensure!(read_stages(&data, &scope, &tag)? == [WaitingForProjectOwnership]);
            ensure!(feed.push(OpeningDatabase));
            written.wait_for(|written| *written == 2).await?;
            ensure!(
                read_stages(&data, &scope, &tag)? == [WaitingForProjectOwnership, OpeningDatabase]
            );
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("the publisher did not write within ten seconds")??;
        retire(feed.finish(), &data, &scope).await;
        ensure!(!directory(&data, &scope)?.join(RECORD).exists());
        Ok(())
    }

    // A record write that never completes cannot delay the open.
    #[tokio::test]
    async fn a_stalled_publisher_never_delays_the_open() -> Result<()> {
        warm_runtime_cache().await?;
        let deadline = fixture_deadline(1, 0);
        tokio::time::timeout(deadline, async {
            let root = tempfile::tempdir()?;
            let (_project, scope, data, mut options) = owner_fixture(root.path())?;
            options.starter_token = Some(Uuid::new_v4());
            let _gate = crate::spawn_gate::spawning().await;
            let gate = Arc::new(tokio::sync::Semaphore::new(0));
            let hooks = OwnerHooks {
                writes: Writes {
                    gate: Some(Arc::clone(&gate)),
                    ..Writes::default()
                },
                ..OwnerHooks::default()
            };
            let (store, publisher) = open_owner_store(options, hooks).await?;
            let publisher = publisher.context("a tokened owner started no publisher")?;
            ensure!(
                !directory(&data, &scope)?.join(RECORD).exists(),
                "a write passed the closed gate"
            );
            gate.close();
            retire(publisher, &data, &scope).await;
            store.close().await
        })
        .await
        .with_context(|| format!("stalled publisher fixture exceeded its {deadline:?} deadline"))?
    }

    #[tokio::test]
    async fn an_untokened_owner_publishes_no_record() -> Result<()> {
        warm_runtime_cache().await?;
        let deadline = fixture_deadline(1, 0);
        tokio::time::timeout(deadline, async {
            let root = tempfile::tempdir()?;
            let (project, scope, data, options) = owner_fixture(root.path())?;
            ensure!(options.starter_token.is_none());
            let _gate = crate::spawn_gate::spawning().await;
            let owner = ServiceOwner::open(options, &project).await?;
            ensure!(
                owner.activity.is_none(),
                "an untokened owner started a publisher"
            );
            ensure!(!directory(&data, &scope)?.join(RECORD).exists());
            owner.close().await
        })
        .await
        .with_context(|| format!("untokened owner fixture exceeded its {deadline:?} deadline"))?
    }

    #[tokio::test]
    async fn the_record_is_retired_after_the_endpoint_and_before_the_store_closes() -> Result<()> {
        warm_runtime_cache().await?;
        let deadline = fixture_deadline(1, 0);
        tokio::time::timeout(deadline, async {
            let root = tempfile::tempdir()?;
            let (project, scope, data, mut options) = owner_fixture(root.path())?;
            let token = Uuid::new_v4();
            options.starter_token = Some(token);
            let tag = activity_tag(&token);
            let _gate = crate::spawn_gate::spawning().await;
            let owner = ServiceOwner::open(options.clone(), &project).await?;
            owner
                .activity
                .as_ref()
                .context("a tokened owner kept no publisher")?
                .settled()
                .await?;
            ensure!(!read_stages(&data, &scope, &tag)?.is_empty());
            let pause =
                ClosePause::at_each(&[ClosePoint::AfterEndpointRetire, ClosePoint::AfterReap]);
            let (mut knobs, _events) = observed(Admission::AnyAttachment, None);
            knobs.close_pause = Some(pause.clone());
            let served = tokio::spawn(owner.serve_with(knobs));
            drop(attach_raw(&data, &scope, None).await?);
            pause.entered.notified().await;
            ensure!(EndpointRecord::read(&data, &scope)?.is_none());
            ensure!(
                read_stages(&data, &scope, &tag).is_ok(),
                "the record was retired before the endpoint"
            );
            pause.release.notify_one();
            pause.entered.notified().await;
            ensure!(
                !directory(&data, &scope)?.join(RECORD).exists(),
                "the record outlived the store close"
            );
            ensure!(
                !owner_lock_free(&options)?,
                "the owner lock was released before the record was retired"
            );
            pause.release.notify_one();
            served.await??;
            ensure!(owner_lock_free(&options)?);
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| format!("record retirement fixture exceeded its {deadline:?} deadline"))?
    }

    /// Open an owner holding at `stage`, assert its record holds that stage,
    /// then release it and return the failed open's error.
    async fn failed_open_after_publishing(
        options: crate::store::OpenOptions,
        project: PathBuf,
        stage: MemoryOpenStage,
    ) -> Result<anyhow::Error> {
        let token = options
            .starter_token
            .context("fixture owner has no token")?;
        let (data, scope) = (options.data_dir.clone(), options.project_scope.clone());
        let barrier = Arc::new(OpenHold::default());
        let hooks = OwnerHooks {
            hold: Some((stage, Arc::clone(&barrier))),
            ..OwnerHooks::default()
        };
        let mut opening =
            Box::pin(
                async move { ServiceOwner::open_with_activity(options, &project, hooks).await },
            );
        tokio::select! {
            biased;
            () = barrier.entered.notified() => {}
            opened = opening.as_mut() => bail!(
                "the open ended before holding at {stage:?}: {:?}",
                opened.map(|_| ())
            ),
        }
        ensure!(
            read_stages(&data, &scope, &activity_tag(&token))?.contains(&stage),
            "the held open had not published {stage:?}"
        );
        barrier.release.notify_one();
        let error = opening
            .await
            .err()
            .context("the fixture owner opened despite its fault")?;
        ensure!(
            !directory(&data, &scope)?.join(RECORD).exists(),
            "a failed open left its record"
        );
        Ok(error)
    }

    #[tokio::test]
    async fn a_failed_endpoint_publication_retires_the_record() -> Result<()> {
        warm_runtime_cache().await?;
        let deadline = fixture_deadline(1, 0);
        tokio::time::timeout(deadline, async {
            let root = tempfile::tempdir()?;
            let (project, scope, data, mut options) = owner_fixture(root.path())?;
            options.starter_token = Some(Uuid::new_v4());
            // A directory where the endpoint record would be published.
            let services = crate::files::ensure_private_directory(&directory(&data, &scope)?)?;
            std::fs::create_dir(services.path().join("endpoint.json"))?;
            let _gate = crate::spawn_gate::spawning().await;
            failed_open_after_publishing(options.clone(), project, PreparingDatabase).await?;
            ensure!(owner_lock_free(&options)?);
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| format!("failed publication fixture exceeded its {deadline:?} deadline"))?
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_failed_store_open_retires_the_record() -> Result<()> {
        use std::os::unix::fs::PermissionsExt as _;
        let deadline = fixture_deadline(1, 0);
        tokio::time::timeout(deadline, async {
            let root = tempfile::tempdir()?;
            let (project, _scope, _data, mut options) = owner_fixture(root.path())?;
            options.starter_token = Some(Uuid::new_v4());
            // An engine whose version probe fails after the probe is reported.
            let engine = root.path().join("failing-engine");
            std::fs::write(&engine, "#!/bin/sh\nexit 1\n")?;
            std::fs::set_permissions(&engine, std::fs::Permissions::from_mode(0o700))?;
            options.config.dolt_binary = Some(engine);
            let _gate = crate::spawn_gate::spawning().await;
            failed_open_after_publishing(options.clone(), project, CheckingRuntimeVersion).await?;
            ensure!(owner_lock_free(&options)?);
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| format!("failed store open fixture exceeded its {deadline:?} deadline"))?
    }

    // T4 integration half, forwarding through a held owner, then T3.
    #[tokio::test]
    async fn a_starter_forwards_only_its_own_owners_stages() -> Result<()> {
        warm_runtime_cache().await?;
        let deadline = fixture_deadline(1, 1);
        tokio::time::timeout(deadline, async {
            let root = tempfile::tempdir()?;
            let (project, scope, data, options) = owner_fixture(root.path())?;
            let executable = options
                .supervisor
                .clone()
                .context("fixture supervisor absent")?;
            let holds = root.path().join("holds");
            std::fs::create_dir(&holds)?;
            std::fs::write(holds.join("PreparingDatabase.hold"), b"")?;
            write_record(
                &directory(&data, &scope)?,
                &activity_tag(&Uuid::new_v4()),
                &[UpgradingDatabase],
            )?;
            let environment = vec![(OPEN_HOLD_DIR_ENV.into(), holds.clone().into_os_string())];
            let mut seen = Vec::new();
            let memory = with_owner_environment(environment, async {
                let _gate = crate::spawn_gate::spawning().await;
                let (mut progress, opening) = crate::MemoryStore::open_managed_observed(
                    options.clone(),
                    project.clone(),
                    executable.clone(),
                );
                let mut opening = Box::pin(opening);
                until_stage(&mut progress, &mut opening, PreparingDatabase, &mut seen).await?;
                std::fs::remove_file(holds.join("PreparingDatabase.hold"))?;
                finish(progress, opening, &mut seen).await
            })
            .await?;
            ensure!(seen.first() == Some(&StartingMemoryService), "{seen:?}");
            ensure!(
                !seen.contains(&UpgradingDatabase),
                "a foreign record was presented: {seen:?}"
            );
            ensure!(!seen.contains(&WaitingForProjectOwnership), "{seen:?}");
            assert_ready_once_and_last(&seen)?;
            memory.close().await?;
            await_managed_quiescence(&options).await?;

            // T3: an existing current store with no owner running.
            let _gate = crate::spawn_gate::spawning().await;
            let (progress, opening) =
                crate::MemoryStore::open_managed_observed(options.clone(), project, executable);
            let mut seen = Vec::new();
            let memory = finish(progress, Box::pin(opening), &mut seen).await?;
            ensure!(seen.first() == Some(&StartingMemoryService), "{seen:?}");
            for absent in [
                CreatingDatabase,
                UpgradingDatabase,
                WaitingForProjectOwnership,
            ] {
                ensure!(!seen.contains(&absent), "{absent:?} on a reopen: {seen:?}");
            }
            assert_ready_once_and_last(&seen)?;
            memory.close().await?;
            await_managed_quiescence(&options).await
        })
        .await
        .with_context(|| format!("forwarding fixture exceeded its {deadline:?} deadline"))?
    }

    /// Spawn a real owner for `options` held at `stage` by the test-support
    /// hold, wait until that stage is received, release the hold and finish.
    /// Returns every stage the client saw and the open store.
    async fn open_held_at(
        options: &crate::store::OpenOptions,
        project: &Path,
        root: &Path,
        stage: MemoryOpenStage,
    ) -> Result<(Vec<MemoryOpenStage>, crate::MemoryStore)> {
        let executable = options
            .supervisor
            .clone()
            .context("fixture supervisor absent")?;
        let holds = root.join("holds");
        std::fs::create_dir(&holds)?;
        let hold = holds.join(format!("{stage:?}.hold"));
        std::fs::write(&hold, b"")?;
        let environment = vec![(OPEN_HOLD_DIR_ENV.into(), holds.into_os_string())];
        let mut seen = Vec::new();
        let memory = with_owner_environment(environment, async {
            let _gate = crate::spawn_gate::spawning().await;
            let (mut progress, opening) = crate::MemoryStore::open_managed_observed(
                options.clone(),
                project.to_owned(),
                executable,
            );
            let mut opening = Box::pin(opening);
            until_stage(&mut progress, &mut opening, stage, &mut seen).await?;
            std::fs::remove_file(&hold)?;
            finish(progress, opening, &mut seen).await
        })
        .await?;
        Ok((seen, memory))
    }

    // T1: the owner reports creation, and the starter forwards it, for a
    // project that has no memory yet.
    #[tokio::test]
    async fn a_starter_forwards_the_creation_of_a_new_project() -> Result<()> {
        warm_runtime_cache().await?;
        let deadline = fixture_deadline(1, 1);
        tokio::time::timeout(deadline, async {
            let root = tempfile::tempdir()?;
            let (project, _scope, _data, options) = owner_fixture(root.path())?;
            let (seen, memory) =
                open_held_at(&options, &project, root.path(), CreatingDatabase).await?;
            let starting = seen
                .iter()
                .position(|stage| *stage == StartingMemoryService);
            let creating = seen.iter().position(|stage| *stage == CreatingDatabase);
            ensure!(
                starting.is_some() && starting < creating,
                "creation was not forwarded after the service start: {seen:?}"
            );
            ensure!(
                seen.iter()
                    .filter(|stage| **stage == CreatingDatabase)
                    .count()
                    == 1,
                "{seen:?}"
            );
            for absent in [UpgradingDatabase, WaitingForProjectOwnership] {
                ensure!(
                    !seen.contains(&absent),
                    "{absent:?} for a new project: {seen:?}"
                );
            }
            assert_ready_once_and_last(&seen)?;
            memory.close().await?;
            await_managed_quiescence(&options).await
        })
        .await
        .with_context(|| format!("creation fixture exceeded its {deadline:?} deadline"))?
    }

    // T2: a store left at its released format is upgraded, and the starter
    // forwards the upgrade, not a creation.
    #[tokio::test]
    async fn a_starter_forwards_the_upgrade_of_a_released_store() -> Result<()> {
        warm_runtime_cache().await?;
        let deadline = fixture_deadline(1, 1);
        tokio::time::timeout(deadline, async {
            let root = tempfile::tempdir()?;
            let (project, _scope, _data, options) = owner_fixture(root.path())?;
            {
                let _gate = crate::spawn_gate::spawning().await;
                crate::store::released_v1(&options).await?;
            }
            let (seen, memory) =
                open_held_at(&options, &project, root.path(), UpgradingDatabase).await?;
            ensure!(
                seen.iter()
                    .filter(|stage| **stage == UpgradingDatabase)
                    .count()
                    == 1,
                "{seen:?}"
            );
            for absent in [CreatingDatabase, WaitingForProjectOwnership] {
                ensure!(
                    !seen.contains(&absent),
                    "{absent:?} for an upgrade: {seen:?}"
                );
            }
            assert_ready_once_and_last(&seen)?;
            memory.close().await?;
            await_managed_quiescence(&options).await
        })
        .await
        .with_context(|| format!("upgrade fixture exceeded its {deadline:?} deadline"))?
    }

    // The owner, not the starter, meets a busy project startup lock: it
    // reports the wait, the starter forwards it after its own service start,
    // and the wait ends with the lock.
    #[tokio::test]
    async fn a_starter_forwards_its_owners_wait_for_the_project_startup_lock() -> Result<()> {
        warm_runtime_cache().await?;
        let deadline = fixture_deadline(1, 1);
        tokio::time::timeout(deadline, async {
            let root = tempfile::tempdir()?;
            let (project, _scope, _data, options) = owner_fixture(root.path())?;
            let executable = options
                .supervisor
                .clone()
                .context("fixture supervisor absent")?;
            let _gate = crate::spawn_gate::spawning().await;
            let held = crate::store::hold_startup_lock(&options)?;
            let (mut progress, opening) =
                crate::MemoryStore::open_managed_observed(options.clone(), project, executable);
            let mut opening = Box::pin(opening);
            let mut seen = Vec::new();
            until_stage(
                &mut progress,
                &mut opening,
                WaitingForProjectOwnership,
                &mut seen,
            )
            .await?;
            ensure!(
                seen == [StartingMemoryService, WaitingForProjectOwnership],
                "{seen:?}"
            );
            drop(held);
            let memory = finish(progress, opening, &mut seen).await?;
            ensure!(
                seen.iter()
                    .filter(|stage| **stage == WaitingForProjectOwnership)
                    .count()
                    == 1,
                "{seen:?}"
            );
            assert_ready_once_and_last(&seen)?;
            memory.close().await?;
            await_managed_quiescence(&options).await
        })
        .await
        .with_context(|| format!("owner wait fixture exceeded its {deadline:?} deadline"))?
    }

    // T5
    #[tokio::test]
    async fn a_failing_publisher_leaves_the_open_and_the_starters_own_stages() -> Result<()> {
        warm_runtime_cache().await?;
        let deadline = fixture_deadline(1, 0);
        tokio::time::timeout(deadline, async {
            let root = tempfile::tempdir()?;
            let (project, scope, data, options) = owner_fixture(root.path())?;
            let executable = options
                .supervisor
                .clone()
                .context("fixture supervisor absent")?;
            let environment = vec![(WRITE_FAILURE_ENV.into(), OsString::from("1"))];
            let mut seen = Vec::new();
            let memory = with_owner_environment(environment, async {
                let _gate = crate::spawn_gate::spawning().await;
                let (progress, opening) =
                    crate::MemoryStore::open_managed_observed(options.clone(), project, executable);
                finish(progress, Box::pin(opening), &mut seen).await
            })
            .await?;
            ensure!(seen == [StartingMemoryService, Ready], "{seen:?}");
            ensure!(!directory(&data, &scope)?.join(RECORD).exists());
            memory.close().await?;
            await_managed_quiescence(&options).await
        })
        .await
        .with_context(|| format!("failing publisher fixture exceeded its {deadline:?} deadline"))?
    }

    // T6
    #[tokio::test]
    async fn a_starter_waiting_for_owner_authority_reports_the_wait_then_its_start() -> Result<()> {
        warm_runtime_cache().await?;
        let deadline = fixture_deadline(1, 0);
        tokio::time::timeout(deadline, async {
            let root = tempfile::tempdir()?;
            let (project, scope, data, options) = owner_fixture(root.path())?;
            let executable = options
                .supervisor
                .clone()
                .context("fixture supervisor absent")?;
            let _gate = crate::spawn_gate::spawning().await;
            let held = ServiceLock::try_acquire(&data, &scope, ServiceLockKind::Owner)?
                .context("fixture did not acquire the owner lock")?;
            let (mut progress, opening) =
                crate::MemoryStore::open_managed_observed(options.clone(), project, executable);
            let mut opening = Box::pin(opening);
            let mut seen = Vec::new();
            until_stage(
                &mut progress,
                &mut opening,
                WaitingForProjectOwnership,
                &mut seen,
            )
            .await?;
            held.release()?;
            let memory = finish(progress, opening, &mut seen).await?;
            ensure!(
                seen.starts_with(&[WaitingForProjectOwnership, StartingMemoryService]),
                "{seen:?}"
            );
            assert_ready_once_and_last(&seen)?;
            memory.close().await?;
            await_managed_quiescence(&options).await
        })
        .await
        .with_context(|| format!("waiting starter fixture exceeded its {deadline:?} deadline"))?
    }

    // T6b and T7: a read-only inspection waits, then opens locally with an
    // empty private engine cache and forwards its own stages.
    #[tokio::test]
    async fn a_waiting_inspection_forwards_its_own_local_stages() -> Result<()> {
        warm_runtime_cache().await?;
        // One cold engine installation beyond the fixture's own opens.
        let deadline = fixture_deadline(1, 2);
        tokio::time::timeout(deadline, async {
            let root = tempfile::tempdir()?;
            let (project, scope, data, options) = owner_fixture(root.path())?;
            let executable = options
                .supervisor
                .clone()
                .context("fixture supervisor absent")?;
            let _gate = crate::spawn_gate::spawning().await;
            crate::store::MemoryStore::open(options.clone())
                .await?
                .close()
                .await?;
            let mut inspection = options.clone();
            inspection.read_only = true;
            inspection.config.cache_dir = Some(root.path().join("empty-cache"));
            let held = ServiceLock::try_acquire(&data, &scope, ServiceLockKind::Owner)?
                .context("fixture did not acquire the owner lock")?;
            let (mut progress, opening) =
                crate::MemoryStore::open_managed_observed(inspection, project, executable);
            let mut opening = Box::pin(opening);
            let mut seen = Vec::new();
            until_stage(
                &mut progress,
                &mut opening,
                WaitingForProjectOwnership,
                &mut seen,
            )
            .await?;
            held.release()?;
            let memory = finish(progress, opening, &mut seen).await?;
            ensure!(
                seen.first() == Some(&WaitingForProjectOwnership),
                "{seen:?}"
            );
            ensure!(
                !seen.contains(&StartingMemoryService),
                "an inspection started an owner"
            );
            let extracting = seen
                .iter()
                .position(|stage| *stage == ExtractingEmbeddedRuntime)
                .with_context(|| format!("the local open did not report extraction: {seen:?}"))?;
            ensure!(
                seen[extracting..].contains(&PreparingDatabase),
                "the local open reported preparation before extraction: {seen:?}"
            );
            assert_ready_once_and_last(&seen)?;
            memory.close().await
        })
        .await
        .with_context(|| format!("waiting inspection fixture exceeded its {deadline:?} deadline"))?
    }

    // T19: the previous owner has retired its endpoint and still holds its
    // lock while it reaps.
    #[tokio::test]
    async fn a_starter_meeting_a_closing_owner_waits_then_starts_its_own() -> Result<()> {
        warm_runtime_cache().await?;
        let deadline = fixture_deadline(1, 1);
        tokio::time::timeout(deadline, async {
            let root = tempfile::tempdir()?;
            let (project, scope, data, options) = owner_fixture(root.path())?;
            let executable = options
                .supervisor
                .clone()
                .context("fixture supervisor absent")?;
            let _gate = crate::spawn_gate::spawning().await;
            let owner = ServiceOwner::open(options.clone(), &project).await?;
            let pause = ClosePause::at(ClosePoint::AfterEndpointRetire);
            let (mut knobs, _events) = observed(Admission::AnyAttachment, None);
            knobs.close_pause = Some(pause.clone());
            let served = tokio::spawn(owner.serve_with(knobs));
            drop(attach_raw(&data, &scope, None).await?);
            pause.entered.notified().await;
            let (mut progress, opening) =
                crate::MemoryStore::open_managed_observed(options.clone(), project, executable);
            let mut opening = Box::pin(opening);
            let mut seen = Vec::new();
            until_stage(
                &mut progress,
                &mut opening,
                WaitingForProjectOwnership,
                &mut seen,
            )
            .await?;
            pause.release.notify_one();
            let memory = finish(progress, opening, &mut seen).await?;
            served.await??;
            ensure!(
                seen.starts_with(&[WaitingForProjectOwnership, StartingMemoryService]),
                "{seen:?}"
            );
            assert_ready_once_and_last(&seen)?;
            memory.close().await?;
            await_managed_quiescence(&options).await
        })
        .await
        .with_context(|| format!("closing owner fixture exceeded its {deadline:?} deadline"))?
    }
}
