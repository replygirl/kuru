//! The open-activity record: the open stages an owner's own observed open has
//! begun, published beside its endpoint so the client that started it can say
//! what Kuru is doing before that endpoint exists.
//!
//! The record also carries the open's own progress count, which advances only
//! at bounded one-shot points of that open, and, once an open that will fail
//! is about to close the engine it started, a failing mark with the failure's
//! own text. Its starter's readiness wait may only be extended by a change of
//! the record and only ended by its retirement or that mark.
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
use crate::progress::{MemoryOpenStage, OpenAdvance, ProgressReporter};
use crate::store::{MemoryStore, OpenOptions};

const RECORD: &str = "activity.json";
/// Private stage for a record leaving its name; never read as a record.
const RETIRED_RECORD: &str = "activity.retired";
const RECORD_LIMIT: u64 = 4 * 1024;
/// Format 2 adds the progress count and the failing mark; format 1 is not
/// read. One binary is both client and owner, so a mismatch arises only from
/// an in-place replacement between a client's start and its spawn.
const FORMAT: u32 = 2;
/// The failure reason's bound in bytes, before JSON escaping. Control
/// characters become spaces first, so escaping at most doubles it and the
/// whole record stays within [`RECORD_LIMIT`].
const REASON_LIMIT: usize = 1024;
/// Stands in for a starter token or tag found in a failure reason.
const REDACTED: &str = "[redacted]";
/// Separates this tag from any other digest of the same token.
const TAG_CONTEXT: &[u8] = b"kuru-open-activity-v1\0";
/// After a failed write the publisher waits this long, or for a newer list,
/// then writes the latest list again. It never retries the open.
const REWRITE_AFTER: Duration = Duration::from_millis(100);
/// A change of the progress count alone is written at most this often; the
/// latest count is always written once the spacing has passed. A stage
/// change or the failing mark is written at once.
const PROGRESS_SPACING: Duration = Duration::from_millis(250);

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
pub(super) fn stage_name(stage: MemoryOpenStage) -> Option<&'static str> {
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
    progress: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    failing: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

/// What one owner's record says about its open.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Activity {
    /// The published stages its open has begun, in order.
    pub(crate) stages: Vec<MemoryOpenStage>,
    /// The open's progress count.
    pub(crate) progress: u64,
    /// The failure reason, present exactly when the open is marked failing.
    pub(crate) failure: Option<String>,
}

/// `text` with control characters as spaces, cut at a character boundary to
/// at most [`REASON_LIMIT`] bytes.
fn bounded_reason(text: &str) -> String {
    let mut reason = String::with_capacity(text.len().min(REASON_LIMIT));
    for character in text.chars() {
        let character = if character.is_control() {
            ' '
        } else {
            character
        };
        if reason.len() + character.len_utf8() > REASON_LIMIT {
            break;
        }
        reason.push(character);
    }
    reason
}

fn encode(tag: &str, activity: &Activity) -> Result<Vec<u8>> {
    let stages = activity
        .stages
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
        progress: activity.progress,
        failing: activity.failure.is_some(),
        reason: activity.failure.as_deref().map(bounded_reason),
    })?;
    ensure!(
        bytes.len() as u64 <= RECORD_LIMIT,
        "open activity record exceeds its limit"
    );
    Ok(bytes)
}

/// A record's tag and activity. An unknown field, format, tag shape or stage
/// name, a failing mark without its reason or a reason without the mark
/// makes the whole record unreadable.
fn decode(bytes: &[u8]) -> Result<(String, Activity)> {
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
    ensure!(
        record.failing == record.reason.is_some(),
        "open activity record has a failing mark without its reason"
    );
    let stages = record
        .stages
        .iter()
        .map(|name| named_stage(name).context("unknown open stage in activity record"))
        .collect::<Result<_>>()?;
    Ok((
        record.tag,
        Activity {
            stages,
            progress: record.progress,
            failure: record.reason,
        },
    ))
}

/// The owner-private directory that also holds the endpoint record.
fn directory(data_dir: &Path, scope: &str) -> Result<PathBuf> {
    EndpointRecord::directory(data_dir, scope)
}

fn write_record(directory: &Path, tag: &str, activity: &Activity) -> Result<()> {
    let bytes = encode(tag, activity)?;
    crate::files::ensure_private_directory(directory)?;
    crate::files::write(&directory.join(RECORD), &bytes)
}

/// Test-only: publish a record for `tag` as its owner would, with `stages`
/// and the progress count `progress`, through the same encoding and private
/// staged write.
#[cfg(test)]
pub(super) fn write_progress_record(
    data_dir: &Path,
    scope: &str,
    tag: &str,
    stages: &[MemoryOpenStage],
    progress: u64,
) -> Result<()> {
    write_record(
        &directory(data_dir, scope)?,
        tag,
        &Activity {
            stages: stages.to_vec(),
            progress,
            failure: None,
        },
    )
}

/// Test-only: [`write_progress_record`] with the failing mark and `reason`.
#[cfg(test)]
pub(super) fn write_failing_record(
    data_dir: &Path,
    scope: &str,
    tag: &str,
    stages: &[MemoryOpenStage],
    progress: u64,
    reason: &str,
) -> Result<()> {
    write_record(
        &directory(data_dir, scope)?,
        tag,
        &Activity {
            stages: stages.to_vec(),
            progress,
            failure: Some(reason.to_owned()),
        },
    )
}

/// Test-only: retire the record tagged `tag` exactly as its owner does.
#[cfg(test)]
pub(super) fn retire_tagged_record(data_dir: &Path, scope: &str, tag: &str) -> Result<()> {
    retire_record(&directory(data_dir, scope)?, tag)
}

/// Test-only: the activity of the record tagged `tag`, or why it cannot be
/// read.
#[cfg(test)]
pub(crate) fn read_activity(data_dir: &Path, scope: &str, tag: &str) -> Result<Activity> {
    let bytes = crate::files::read_bytes(&directory(data_dir, scope)?.join(RECORD), RECORD_LIMIT)?;
    let (found, activity) = decode(&bytes)?;
    ensure!(
        found == tag,
        "open activity record belongs to another owner"
    );
    Ok(activity)
}

#[cfg(test)]
fn read_stages(data_dir: &Path, scope: &str, tag: &str) -> Result<Vec<MemoryOpenStage>> {
    read_activity(data_dir, scope, tag).map(|activity| activity.stages)
}

/// What one read of the record's name found.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Observation {
    /// Opening the record's name, or the directory that holds it, found
    /// nothing there.
    Absent,
    /// Something is there that cannot be presented this poll: another
    /// owner's record, an undecodable or oversized one, a record replaced or
    /// retired between its read and the check that its name still holds it,
    /// or any other failure. It decides nothing.
    Unusable,
    /// The record tagged with this starter's tag.
    Record(Activity),
}

/// One bounded read of the record tagged `tag`. Only a missing name is
/// [`Observation::Absent`]; a failed verification after the read is
/// [`Observation::Unusable`] even when the name has since gone, so a
/// retirement is first seen as absent at the next read.
pub(crate) fn observe(data_dir: &Path, scope: &str, tag: &str) -> Observation {
    observe_then(data_dir, scope, tag, || {})
}

/// [`observe`] with a hook between reading the held record and verifying
/// that its name still identifies it.
fn observe_then(data_dir: &Path, scope: &str, tag: &str, between: impl FnOnce()) -> Observation {
    let Ok(path) = directory(data_dir, scope).map(|directory| directory.join(RECORD)) else {
        return Observation::Unusable;
    };
    let (parent, file) = match crate::files::read(&path, Privacy::OwnerOnly) {
        Ok(opened) => opened,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            return Observation::Absent;
        }
        Err(_) => return Observation::Unusable,
    };
    let Ok(bytes) = crate::files::read_held_then(&parent, &path, file, RECORD_LIMIT, between)
    else {
        return Observation::Unusable;
    };
    match decode(&bytes) {
        Ok((found, activity)) if found == tag => Observation::Record(activity),
        _ => Observation::Unusable,
    }
}

/// The elected starter's view of its own owner's record across readiness
/// polls. Progress is a change of the whole record, timed on the starter's
/// own clock when it is first read; the record carries no clock.
pub(crate) struct OwnerWatch {
    tag: String,
    /// The last record read, which every later read is compared with.
    last: Option<Activity>,
    /// The stages already forwarded to an observing reporter.
    forwarded: usize,
    last_progress: tokio::time::Instant,
}

/// What one poll's read of the owner's record decided.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Watched {
    /// The wait goes on: the record changed, is unchanged, unusable or not
    /// yet written.
    Waiting,
    /// A record this starter had read is no longer under its name.
    Retired,
    /// The record is marked failing, with the reason it carries.
    Failing(String),
}

impl OwnerWatch {
    /// A watch of the record tagged `tag`, whose first window starts at
    /// `spawned`.
    pub(crate) fn new(tag: String, spawned: tokio::time::Instant) -> Self {
        Self {
            tag,
            last: None,
            forwarded: 0,
            last_progress: spawned,
        }
    }

    /// Read the record once and forward, in order, the stages an observing
    /// `progress` has not yet received.
    pub(crate) fn poll(
        &mut self,
        data_dir: &Path,
        scope: &str,
        progress: &mut ProgressReporter,
    ) -> Watched {
        let observation = observe(data_dir, scope, &self.tag);
        self.saw(observation, progress)
    }

    fn saw(&mut self, observation: Observation, progress: &mut ProgressReporter) -> Watched {
        match observation {
            Observation::Record(activity) if self.last.as_ref() != Some(&activity) => {
                if progress.is_observed() {
                    for stage in activity.stages.iter().skip(self.forwarded) {
                        progress.report(*stage);
                    }
                    self.forwarded = self.forwarded.max(activity.stages.len());
                }
                let failure = activity.failure.clone();
                self.last = Some(activity);
                // The failing mark ends the wait; it is not progress, so the
                // time since progress stays measured from the change before.
                match failure {
                    Some(reason) => Watched::Failing(reason),
                    None => {
                        self.last_progress = tokio::time::Instant::now();
                        Watched::Waiting
                    }
                }
            }
            Observation::Absent if self.last.is_some() => Watched::Retired,
            Observation::Record(_) | Observation::Absent | Observation::Unusable => {
                Watched::Waiting
            }
        }
    }

    /// The last stage of the last record read.
    pub(crate) fn last_stage(&self) -> Option<MemoryOpenStage> {
        self.last
            .as_ref()
            .and_then(|activity| activity.stages.last().copied())
    }

    /// The owner's progress count in the last record read.
    pub(crate) fn seen(&self) -> u64 {
        self.last.as_ref().map_or(0, |activity| activity.progress)
    }

    /// When this starter first read the last change that was progress, or
    /// the spawn when it has read none.
    pub(crate) fn last_progress(&self) -> tokio::time::Instant {
        self.last_progress
    }
}

/// Test-only: each successful record write and when it completed.
#[cfg(test)]
type WriteLog = watch::Sender<Vec<(tokio::time::Instant, Activity)>>;

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
    /// Every successful write, with the instant it completed.
    #[cfg(test)]
    log: Option<Arc<WriteLog>>,
    /// Record writes in `log` without touching the filesystem.
    #[cfg(test)]
    no_io: bool,
}

impl Writes {
    async fn write(&self, directory: &Path, tag: &str, activity: Activity) -> Result<()> {
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
        #[cfg(test)]
        let logged = activity.clone();
        #[cfg(test)]
        let skip = self.no_io;
        #[cfg(not(test))]
        let skip = false;
        if !skip {
            let (directory, tag) = (directory.to_owned(), tag.to_owned());
            tokio::task::spawn_blocking(move || write_record(&directory, &tag, &activity))
                .await??;
        }
        #[cfg(test)]
        if let Some(log) = &self.log {
            let now = tokio::time::Instant::now();
            log.send_modify(|writes| writes.push((now, logged)));
        }
        Ok(())
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
    /// Pauses the owner's ending before its starter attached, after its
    /// store opened, at that ending's [`super::ClosePoint`]s.
    #[cfg(test)]
    pub(crate) close_pause: Option<Arc<super::ClosePause>>,
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
                    #[cfg(test)]
                    log: None,
                    #[cfg(test)]
                    no_io: false,
                },
                hold_dir: std::env::var_os(OPEN_HOLD_DIR_ENV)
                    .filter(|directory| !directory.is_empty())
                    .map(PathBuf::from),
                #[cfg(test)]
                hold: None,
                #[cfg(test)]
                close_pause: None,
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
    task: tokio::task::JoinHandle<Activity>,
    target: Target,
    /// The length of the last list written.
    #[cfg(any(test, feature = "test-support"))]
    written: watch::Receiver<usize>,
}

/// Where and how one owner's record is written, and the text its failure
/// reason must never carry: the starter token's spellings and the tag.
struct Target {
    directory: PathBuf,
    tag: String,
    redact: Vec<String>,
    writes: Writes,
}

/// `reason` with every non-empty spelling in `redact` replaced.
fn redacted(reason: &str, redact: &[String]) -> String {
    redact
        .iter()
        .filter(|text| !text.is_empty())
        .fold(reason.to_owned(), |reason, text| {
            reason.replace(text.as_str(), REDACTED)
        })
}

/// What the publisher reads from its open, and what it never writes.
struct Source {
    stages: watch::Receiver<Arc<[MemoryOpenStage]>>,
    /// The open's progress count and failing mark; `None` publishes stages
    /// only, as an open that does not count.
    ticks: Option<watch::Receiver<OpenAdvance>>,
    /// Whether the counter can still change; its last value stays readable.
    ticks_live: bool,
    /// Text that must not reach the record: the starter token's spellings
    /// and the tag.
    redact: Vec<String>,
}

impl Feed {
    fn start(
        directory: PathBuf,
        tag: String,
        redact: Vec<String>,
        writes: Writes,
        ticks: Option<watch::Receiver<OpenAdvance>>,
    ) -> Self {
        let (sender, stages) = watch::channel(Arc::<[MemoryOpenStage]>::from([]));
        let (written_sender, _written) = watch::channel(0);
        let source = Source {
            stages,
            ticks_live: ticks.is_some(),
            ticks,
            redact: redact.clone(),
        };
        let task = tokio::spawn(publish(
            directory.clone(),
            tag.clone(),
            source,
            writes.clone(),
            written_sender,
        ));
        Self {
            stages: Vec::new(),
            sender,
            task,
            target: Target {
                directory,
                tag,
                redact,
                writes,
            },
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

    /// End the feed with its open. The publisher may finish the latest
    /// activity, then stops; it never removes the record.
    fn finish(self) -> Publisher {
        drop(self.sender);
        Publisher {
            task: Some(self.task),
            target: self.target,
            #[cfg(test)]
            written: self.written,
            #[cfg(test)]
            published: self.stages.len(),
        }
    }
}

impl Source {
    /// The latest activity, marking both inputs seen.
    fn latest(&mut self) -> Activity {
        let stages = self.stages.borrow_and_update().to_vec();
        let advance = self
            .ticks
            .as_mut()
            .map(|ticks| ticks.borrow_and_update().clone())
            .unwrap_or_default();
        Activity {
            stages,
            progress: advance.count,
            failure: advance
                .failure
                .map(|reason| redacted(&reason, &self.redact)),
        }
    }

    /// Wait for either input to change. A closed counter is no longer
    /// waited for; a closed stage feed returns at once, so the caller sees
    /// that its open has ended.
    async fn changed(&mut self) {
        let live = self.ticks_live;
        let ticks = async {
            match self.ticks.as_mut() {
                Some(ticks) if live => ticks.changed().await.is_ok(),
                _ => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = self.stages.changed() => {}
            live = ticks => self.ticks_live = live,
        }
    }
}

/// Writes the latest activity on its own task, so nothing in the open,
/// endpoint publication or serving waits for a write. A stage change or the
/// failing mark is written at once; a change of the progress count alone at
/// most every [`PROGRESS_SPACING`], always followed by the latest count. When
/// the open has ended it writes any unwritten activity once and stops,
/// returning that latest activity whether or not its write succeeded.
async fn publish(
    directory: PathBuf,
    tag: String,
    mut source: Source,
    writes: Writes,
    written: watch::Sender<usize>,
) -> Activity {
    // The record a reader can see, and the last activity this task tried to
    // write; a failed attempt waits for a newer stage or failing mark, or
    // for `REWRITE_AFTER`.
    let mut published = Activity::default();
    let mut attempted = Activity::default();
    let mut not_before: Option<tokio::time::Instant> = None;
    loop {
        let latest = source.latest();
        // `has_changed` fails only once the open has dropped its feed;
        // `latest` already holds a list sent just before that.
        let ended = source.stages.has_changed().is_err();
        if latest == published {
            if ended {
                return latest;
            }
        } else {
            let urgent = latest.stages != attempted.stages || latest.failure != attempted.failure;
            let due = not_before.is_none_or(|at| tokio::time::Instant::now() >= at);
            if ended || urgent || due {
                attempted = latest.clone();
                let stages = latest.stages.len();
                if writes.write(&directory, &tag, latest.clone()).await.is_ok() {
                    published = latest;
                    written.send_replace(stages);
                    not_before = Some(tokio::time::Instant::now() + PROGRESS_SPACING);
                } else if ended {
                    return latest;
                } else {
                    // The latest activity holds every earlier one, so writing
                    // it again repairs whatever a reader missed.
                    not_before = Some(tokio::time::Instant::now() + REWRITE_AFTER);
                }
                continue;
            }
        }
        let pending = latest != published;
        tokio::select! {
            () = source.changed() => {}
            () = async {
                match not_before {
                    Some(at) if pending => tokio::time::sleep_until(at).await,
                    _ => std::future::pending().await,
                }
            } => {}
        }
    }
}

/// The finished publisher of one owner's record, kept until the owner retires
/// that record within its own close or failed open.
pub(crate) struct Publisher {
    /// The publisher's task until it is joined, by a failing mark or by
    /// retirement.
    task: Option<tokio::task::JoinHandle<Activity>>,
    target: Target,
    #[cfg(test)]
    written: watch::Receiver<usize>,
    #[cfg(test)]
    published: usize,
}

impl Publisher {
    /// Mark the record failing with `reason`, for an owner whose store opened
    /// but which ends before its starter attached. Joins the publisher, then
    /// writes its last activity once more with the failing mark and `reason`,
    /// redacted and bounded as an open's own mark is, and awaits that write
    /// on this task, so no later write can follow the record's retirement.
    /// Says whether the marked record was written; call only while holding
    /// verified owner authority, since the write replaces whatever record
    /// holds the name.
    pub(crate) async fn mark_failing(&mut self, reason: &str) -> bool {
        let Some(task) = self.task.take() else {
            return false;
        };
        let Ok(last) = task.await else {
            return false;
        };
        let Target {
            directory,
            tag,
            redact,
            writes,
        } = &self.target;
        let marked = Activity {
            failure: Some(redacted(reason, redact)),
            ..last
        };
        writes.write(directory, tag, marked).await.is_ok()
    }
}

#[cfg(test)]
impl Publisher {
    /// Join the publisher's task and return the last activity it saw.
    pub(crate) async fn join(mut self) -> Result<Activity> {
        Ok(self
            .task
            .take()
            .context("the publisher was already joined")?
            .await?)
    }

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

    /// Wait until the publisher's task has returned, so the record holds
    /// the last activity it will write while it lives.
    pub(crate) async fn ended(&self) {
        // The task owns the only sender; the wait ends when it drops.
        let _ = self.written.clone().wait_for(|_| false).await;
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
        let tag = activity_tag(token);
        // A failure reason is the open's own error text; none of these
        // spellings may reach the record.
        let redact = vec![
            token.hyphenated().to_string(),
            token.simple().to_string(),
            tag.clone(),
        ];
        Some((
            directory(&options.data_dir, &options.project_scope).ok()?,
            tag,
            redact,
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
    let (mut progress, opening) = MemoryStore::open_observed(options);
    let mut feed = target.map(|(directory, tag, redact)| {
        Feed::start(
            directory,
            tag,
            redact,
            hooks.writes.clone(),
            progress.ticks(),
        )
    });
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
    let Publisher {
        task,
        target: Target { tag, .. },
        ..
    } = publisher;
    // The task ends with its open. Joining it first means no late write can
    // recreate the record after its retirement; a failing mark has already
    // joined it and awaited its own write.
    if let Some(task) = task {
        let _ = task.await;
    }
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

/// Run `future` with `environment` added to every owner its opens spawn.
#[cfg(test)]
pub(crate) async fn with_owner_environment<F: std::future::Future>(
    environment: Vec<(OsString, OsString)>,
    future: F,
) -> F::Output {
    OWNER_TEST_ENVIRONMENT.scope(environment, future).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryOpenProgress;
    use crate::service::tests::{attach_raw, owner_fixture, owner_lock_free};
    use crate::service::{
        Admission, ClosePause, ClosePoint, ServeEvent, ServiceLock, ServiceLockKind, ServiceOwner,
    };
    use crate::test_support::{
        await_managed_quiescence, fixture_deadline, next_event, observed, warm_runtime_cache,
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

    /// Activity naming only `stages`, as an open that does not count.
    fn staged(stages: &[MemoryOpenStage]) -> Activity {
        Activity {
            stages: stages.to_vec(),
            ..Activity::default()
        }
    }

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
            ensure!(
                encode(&tag, &staged(&[unpublished])).is_err(),
                "{unpublished:?}"
            );
        }
        let (found, activity) = decode(&encode(&tag, &staged(&published))?)?;
        ensure!(found == tag && activity.stages == published, "{activity:?}");
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
                format!(
                    r#"{{"format":2,"tag":"{tag}","stages":["PreparingDatabase"],"progress":3}}"#
                )
                .as_bytes()
            )?
            .1 == Activity {
                stages: vec![PreparingDatabase],
                progress: 3,
                failure: None,
            }
        );
        for invalid in [
            // Format 1 is no longer read, and no later format is guessed at.
            format!(r#"{{"format":1,"tag":"{tag}","stages":["PreparingDatabase"]}}"#),
            format!(r#"{{"format":3,"tag":"{tag}","stages":[],"progress":0}}"#),
            format!(r#"{{"format":2,"tag":"{tag}","stages":[],"progress":0,"token":"{token}"}}"#),
            format!(r#"{{"format":2,"tag":"{tag}","stages":["Teleporting"],"progress":0}}"#),
            format!(r#"{{"format":2,"tag":"{tag}","stages":["Ready"],"progress":0}}"#),
            format!(r#"{{"format":2,"tag":"{tag}","stages":[]}}"#),
            format!(r#"{{"format":2,"tag":"{tag}","stages":[],"progress":-1}}"#),
            format!(r#"{{"format":2,"tag":"{tag}","stages":[],"progress":0,"failing":true}}"#),
            format!(r#"{{"format":2,"tag":"{tag}","stages":[],"progress":0,"reason":"x"}}"#),
            format!(
                r#"{{"format":2,"tag":"{tag}","stages":[],"progress":0,"failing":false,"reason":"x"}}"#
            ),
            format!(r#"{{"format":2,"tag":"{token}","stages":[],"progress":0}}"#),
            format!(r#"{{"format":2,"tag":"{upper}","stages":[],"progress":0}}"#),
            r#"{"format":2,"stages":[],"progress":0}"#.to_owned(),
        ] {
            ensure!(decode(invalid.as_bytes()).is_err(), "{invalid}");
        }
        Ok(())
    }

    #[test]
    fn the_record_round_trips_progress_and_the_failing_mark() -> Result<()> {
        let tag = activity_tag(&Uuid::new_v4());
        for activity in [
            Activity {
                stages: vec![PreparingDatabase, OpeningDatabase],
                progress: 41,
                failure: None,
            },
            Activity {
                stages: vec![CreatingDatabase],
                progress: u64::MAX,
                failure: Some("open active main pool: refused".into()),
            },
        ] {
            let (found, decoded) = decode(&encode(&tag, &activity)?)?;
            ensure!(found == tag && decoded == activity, "{decoded:?}");
        }
        Ok(())
    }

    // A reason is cut at a character boundary, and quotes and backslashes,
    // which escaping doubles, still keep the whole record within its limit.
    #[test]
    fn an_oversized_reason_is_bounded_within_the_record_limit() -> Result<()> {
        let tag = activity_tag(&Uuid::new_v4());
        let stages: Vec<_> = MemoryOpenStage::ALL
            .into_iter()
            .filter(|stage| stage_name(*stage).is_some())
            .collect();
        for (text, expected) in [
            ("é".repeat(3000), "é".repeat(REASON_LIMIT / 2)),
            ("\"".repeat(3000), "\"".repeat(REASON_LIMIT)),
            ("a\nb\u{7}c".to_owned(), "a b c".to_owned()),
            (
                format!("x{}", "東".repeat(400)),
                format!("x{}", "東".repeat(341)),
            ),
        ] {
            let activity = Activity {
                stages: stages.clone(),
                progress: u64::MAX,
                failure: Some(text),
            };
            let bytes = encode(&tag, &activity)?;
            ensure!(bytes.len() as u64 <= RECORD_LIMIT, "{}", bytes.len());
            let (_, decoded) = decode(&bytes)?;
            let reason = decoded.failure.context("the reason was dropped")?;
            ensure!(reason.len() <= REASON_LIMIT, "{}", reason.len());
            ensure!(reason == expected, "{reason}");
        }
        Ok(())
    }

    /// A publisher whose writes are only logged, with `ticks` as its open's
    /// counter.
    fn logged_feed(
        redact: Vec<String>,
    ) -> Result<(
        Feed,
        crate::progress::OpenTicks,
        Arc<WriteLog>,
        tempfile::TempDir,
    )> {
        let (root, data, scope) = record_fixture()?;
        let log = Arc::new(watch::channel(Vec::new()).0);
        let writes = Writes {
            log: Some(Arc::clone(&log)),
            no_io: true,
            ..Writes::default()
        };
        let (ticks, advance) = crate::progress::OpenTicks::new();
        let feed = Feed::start(
            directory(&data, &scope)?,
            activity_tag(&Uuid::new_v4()),
            redact,
            writes,
            Some(advance),
        );
        Ok((feed, ticks, log, root))
    }

    async fn writes_reach(
        log: &WriteLog,
        count: usize,
    ) -> Result<Vec<(tokio::time::Instant, Activity)>> {
        Ok(log
            .subscribe()
            .wait_for(|writes| writes.len() >= count)
            .await?
            .clone())
    }

    // Paused clock: the log's instants are the publisher's own decisions.
    #[tokio::test(start_paused = true)]
    async fn a_publisher_spaces_progress_writes_and_always_writes_the_latest() -> Result<()> {
        let (mut feed, ticks, log, _root) = logged_feed(Vec::new())?;
        let start = tokio::time::Instant::now();
        let at = |writes: &[(tokio::time::Instant, Activity)], index: usize| {
            writes[index].0.duration_since(start)
        };
        // The first change is written at once.
        ticks.advance();
        let writes = writes_reach(&log, 1).await?;
        ensure!(at(&writes, 0).is_zero() && writes[0].1.progress == 1);
        // A stage change is written at once, inside the spacing, with the
        // latest count; the counts before it are coalesced into it.
        for _ in 0..5 {
            ticks.advance();
        }
        ensure!(feed.push(PreparingDatabase));
        let writes = writes_reach(&log, 2).await?;
        ensure!(at(&writes, 1).is_zero(), "{:?}", at(&writes, 1));
        ensure!(writes[1].1.stages == [PreparingDatabase] && writes[1].1.progress == 6);
        // A count alone waits for the spacing, then is written.
        ticks.advance();
        let writes = writes_reach(&log, 3).await?;
        ensure!(at(&writes, 2) == PROGRESS_SPACING && writes[2].1.progress == 7);
        // Several counts inside one spacing: only the latest is written.
        ticks.advance();
        ticks.advance();
        let writes = writes_reach(&log, 4).await?;
        ensure!(at(&writes, 3) == 2 * PROGRESS_SPACING && writes[3].1.progress == 9);
        // The failing mark is written at once, inside the spacing.
        ticks.mark_failing(&anyhow::anyhow!("refused"));
        let writes = writes_reach(&log, 5).await?;
        ensure!(at(&writes, 4) == 2 * PROGRESS_SPACING);
        ensure!(writes[4].1.failure.as_deref() == Some("refused"));
        // A count still unwritten when the open ends is written at once.
        ticks.advance();
        let publisher = feed.finish();
        publisher.join().await?;
        let writes = log.borrow().clone();
        ensure!(writes.len() == 6, "{writes:?}");
        ensure!(at(&writes, 5) == 2 * PROGRESS_SPACING && writes[5].1.progress == 10);
        ensure!(writes[5].1.failure.as_deref() == Some("refused"));
        Ok(())
    }

    // Writes stop once the open has ended and its last activity is written:
    // a counter that outlives the open (its server keeps it) changes nothing.
    #[tokio::test(start_paused = true)]
    async fn a_publisher_ends_with_its_open_while_its_counter_lives() -> Result<()> {
        let (mut feed, ticks, log, _root) = logged_feed(Vec::new())?;
        ensure!(feed.push(OpeningDatabase));
        writes_reach(&log, 1).await?;
        let publisher = feed.finish();
        publisher.join().await?;
        ticks.advance();
        ensure!(log.borrow().len() == 1, "{:?}", log.borrow());
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn a_failure_reason_never_carries_the_starter_token_or_tag() -> Result<()> {
        let token = Uuid::new_v4();
        let tag = activity_tag(&token);
        let redact = vec![
            token.hyphenated().to_string(),
            token.simple().to_string(),
            tag.clone(),
        ];
        let (feed, ticks, log, _root) = logged_feed(redact)?;
        ticks.mark_failing(&anyhow::anyhow!(
            "starter {token} ({}) tagged {tag} refused",
            token.simple()
        ));
        let writes = writes_reach(&log, 1).await?;
        let reason = writes[0].1.failure.clone().context("no reason written")?;
        ensure!(
            reason == format!("starter {REDACTED} ({REDACTED}) tagged {REDACTED} refused"),
            "{reason}"
        );
        feed.finish().join().await?;
        Ok(())
    }

    // T4, unit half; T9 wrong tag, raw token and size limit.
    #[test]
    fn a_client_presents_only_the_record_of_its_own_owner() -> Result<()> {
        let (_root, data, scope) = record_fixture()?;
        let directory = directory(&data, &scope)?;
        let own = activity_tag(&Uuid::new_v4());
        let (mut progress, mut reporter) = ProgressReporter::observed();
        let mut watch = OwnerWatch::new(own.clone(), tokio::time::Instant::now());
        ensure!(watch.poll(&data, &scope, &mut reporter) == Watched::Waiting);
        ensure!(watch.last.is_none(), "a missing record moved the client");

        // Another owner's record, written by the publisher's own code.
        let token = Uuid::new_v4();
        write_record(
            &directory,
            &activity_tag(&token),
            &staged(&[UpgradingDatabase]),
        )?;
        ensure!(observe(&data, &scope, &own) == Observation::Unusable);
        ensure!(watch.poll(&data, &scope, &mut reporter) == Watched::Waiting);
        ensure!(watch.last.is_none(), "a foreign record moved the client");
        // A record carrying the raw token instead of its tag.
        crate::files::write(
            &directory.join(RECORD),
            format!(
                r#"{{"format":2,"tag":"{token}","stages":["UpgradingDatabase"],"progress":1}}"#
            )
            .as_bytes(),
        )?;
        for tag in [activity_tag(&token), token.to_string()] {
            ensure!(
                observe(&data, &scope, &tag) == Observation::Unusable,
                "{tag}"
            );
        }
        ensure!(watch.poll(&data, &scope, &mut reporter) == Watched::Waiting);
        ensure!(watch.last.is_none(), "a raw-token record moved the client");
        // A record of its own owner over the size limit.
        let mut oversized = encode(&own, &staged(&[UpgradingDatabase]))?;
        oversized.resize(RECORD_LIMIT as usize + 1, b' ');
        crate::files::write(&directory.join(RECORD), &oversized)?;
        ensure!(observe(&data, &scope, &own) == Observation::Unusable);
        ensure!(watch.poll(&data, &scope, &mut reporter) == Watched::Waiting);
        ensure!(watch.last.is_none(), "an oversized record moved the client");
        ensure!(received(&mut progress).is_empty());

        write_record(&directory, &own, &staged(&[PreparingDatabase]))?;
        ensure!(watch.poll(&data, &scope, &mut reporter) == Watched::Waiting);
        ensure!(watch.forwarded == 1 && watch.last_stage() == Some(PreparingDatabase));
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
            let mut watch = OwnerWatch::new(tag.clone(), tokio::time::Instant::now());
            for written in 1..=SAMPLE.len() {
                write_record(&directory, &tag, &staged(&SAMPLE[..written]))?;
                if reads & (1 << (written - 1)) != 0 {
                    watch.poll(&data, &scope, &mut reporter);
                    ensure!(watch.forwarded == written, "reads {reads:#b}");
                }
            }
            watch.poll(&data, &scope, &mut reporter);
            ensure!(watch.forwarded == SAMPLE.len(), "reads {reads:#b}");
            ensure!(received(&mut progress) == SAMPLE, "reads {reads:#b}");
        }
        Ok(())
    }

    // Only a missing name is absent. A record replaced or retired between
    // its read and the check that its name still holds it is unusable for
    // that read, and a retirement is then absent at the next read.
    #[test]
    fn only_a_missing_name_is_an_absent_record() -> Result<()> {
        let (_root, data, scope) = record_fixture()?;
        let tag = activity_tag(&Uuid::new_v4());
        // Neither the directory nor the record exists yet.
        ensure!(observe(&data, &scope, &tag) == Observation::Absent);
        let directory = directory(&data, &scope)?;
        write_record(&directory, &tag, &staged(&[PreparingDatabase]))?;
        ensure!(observe(&data, &scope, &tag) == Observation::Record(staged(&[PreparingDatabase])));
        // A concurrent replacement after the read.
        let replaced = observe_then(&data, &scope, &tag, || {
            write_record(
                &directory,
                &tag,
                &staged(&[PreparingDatabase, OpeningDatabase]),
            )
            .expect("replace the record under the reader");
        });
        ensure!(replaced == Observation::Unusable, "{replaced:?}");
        // A concurrent retirement after the read.
        let retired = observe_then(&data, &scope, &tag, || {
            retire_record(&directory, &tag).expect("retire the record under the reader");
        });
        ensure!(retired == Observation::Unusable, "{retired:?}");
        ensure!(observe(&data, &scope, &tag) == Observation::Absent);
        // An undecodable record of any tag.
        crate::files::write(&directory.join(RECORD), b"{")?;
        ensure!(observe(&data, &scope, &tag) == Observation::Unusable);
        Ok(())
    }

    // Progress is a change of the record, timed when the starter reads it:
    // a stale record with its tag counts once and never again, a lower count
    // after it is a change and counts, and an unusable or absent read in
    // between moves nothing. Retirement ends the wait only after a read.
    #[tokio::test(start_paused = true)]
    async fn a_change_of_the_record_is_progress_and_nothing_else_is() -> Result<()> {
        let (_root, data, scope) = record_fixture()?;
        let directory = directory(&data, &scope)?;
        let tag = activity_tag(&Uuid::new_v4());
        let mut reporter = ProgressReporter::silent();
        let spawned = tokio::time::Instant::now();
        let mut watch = OwnerWatch::new(tag.clone(), spawned);
        let step = Duration::from_millis(10);
        // Nothing written: absent before any read is not a retirement.
        ensure!(watch.poll(&data, &scope, &mut reporter) == Watched::Waiting);
        ensure!(watch.last_progress() == spawned && watch.seen() == 0);
        let progressing = |progress: u64| Activity {
            stages: vec![OpeningDatabase],
            progress,
            failure: None,
        };
        write_record(&directory, &tag, &progressing(80))?;
        tokio::time::advance(step).await;
        let stale = tokio::time::Instant::now();
        ensure!(watch.poll(&data, &scope, &mut reporter) == Watched::Waiting);
        ensure!(watch.last_progress() == stale && watch.seen() == 80);
        tokio::time::advance(step).await;
        ensure!(watch.poll(&data, &scope, &mut reporter) == Watched::Waiting);
        ensure!(
            watch.last_progress() == stale,
            "an unchanged record counted again"
        );
        write_record(&directory, &activity_tag(&Uuid::new_v4()), &progressing(81))?;
        tokio::time::advance(step).await;
        ensure!(watch.poll(&data, &scope, &mut reporter) == Watched::Waiting);
        ensure!(watch.last_progress() == stale, "a foreign record counted");
        write_record(&directory, &tag, &progressing(1))?;
        tokio::time::advance(step).await;
        let renewed = tokio::time::Instant::now();
        ensure!(watch.poll(&data, &scope, &mut reporter) == Watched::Waiting);
        ensure!(
            watch.last_progress() == renewed && watch.seen() == 1,
            "a lower count was not a change"
        );
        retire_record(&directory, &tag)?;
        ensure!(watch.poll(&data, &scope, &mut reporter) == Watched::Retired);
        ensure!(watch.last_stage() == Some(OpeningDatabase) && watch.last_progress() == renewed);
        Ok(())
    }

    // The failing mark ends the wait with its reason, and is not progress.
    #[tokio::test(start_paused = true)]
    async fn a_failing_record_ends_the_wait_with_its_reason() -> Result<()> {
        let (_root, data, scope) = record_fixture()?;
        let directory = directory(&data, &scope)?;
        let tag = activity_tag(&Uuid::new_v4());
        let mut reporter = ProgressReporter::silent();
        let mut watch = OwnerWatch::new(tag.clone(), tokio::time::Instant::now());
        write_record(&directory, &tag, &staged(&[CreatingDatabase]))?;
        ensure!(watch.poll(&data, &scope, &mut reporter) == Watched::Waiting);
        let progressed = watch.last_progress();
        tokio::time::advance(Duration::from_millis(10)).await;
        let failing = Activity {
            stages: vec![CreatingDatabase],
            progress: 2,
            failure: Some("open active main pool: refused".into()),
        };
        write_record(&directory, &tag, &failing)?;
        ensure!(
            watch.poll(&data, &scope, &mut reporter)
                == Watched::Failing("open active main pool: refused".into())
        );
        ensure!(watch.last_stage() == Some(CreatingDatabase) && watch.seen() == 2);
        ensure!(
            watch.last_progress() == progressed,
            "the failing mark counted as progress"
        );
        Ok(())
    }

    #[test]
    fn retirement_removes_only_this_owners_record() -> Result<()> {
        let (_root, data, scope) = record_fixture()?;
        let directory = directory(&data, &scope)?;
        let (own, foreign) = (activity_tag(&Uuid::new_v4()), activity_tag(&Uuid::new_v4()));
        write_record(&directory, &foreign, &staged(&[PreparingDatabase]))?;
        ensure!(retire_record(&directory, &own).is_err());
        ensure!(read_stages(&data, &scope, &foreign)? == [PreparingDatabase]);
        write_record(&directory, &own, &staged(&[PreparingDatabase]))?;
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
        write_record(&directory, &tag, &staged(&[PreparingDatabase]))?;
        let held = crate::files::read(&directory.join(RECORD), Privacy::OwnerOnly)?;
        write_record(
            &directory,
            &tag,
            &staged(&[PreparingDatabase, OpeningDatabase]),
        )?;
        ensure!(read_stages(&data, &scope, &tag)? == [PreparingDatabase, OpeningDatabase]);
        let replacement = crate::files::read(&directory.join(RECORD), Privacy::OwnerOnly)?;
        // Removal of the staged record may wait for these readers.
        retire_record(&directory, &tag)?;
        ensure!(
            read_stages(&data, &scope, &tag).is_err(),
            "the name kept the record"
        );
        write_record(&directory, &tag, &staged(&[CreatingDatabase]))?;
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
        let mut feed = Feed::start(
            directory(&data, &scope)?,
            tag.clone(),
            Vec::new(),
            writes,
            None,
        );
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

    // An open that fails after starting its engine marks its record failing,
    // with the failure's own text, before that engine's close begins, and
    // retires the record after the close.
    #[tokio::test]
    async fn a_failed_open_marks_its_record_failing_before_closing_its_engine() -> Result<()> {
        warm_runtime_cache().await?;
        let deadline = fixture_deadline(1, 0);
        tokio::time::timeout(deadline, async {
            let root = tempfile::tempdir()?;
            let (_project, scope, data, mut options) = owner_fixture(root.path())?;
            let token = Uuid::new_v4();
            options.starter_token = Some(token);
            crate::store::refuse_active_validation(&mut options);
            let log = Arc::new(watch::channel(Vec::new()).0);
            let hooks = OwnerHooks {
                writes: Writes {
                    log: Some(Arc::clone(&log)),
                    ..Writes::default()
                },
                ..OwnerHooks::default()
            };
            let pause = Arc::new(crate::store::failed_open_close::Pause::default());
            let _gate = crate::spawn_gate::spawning().await;
            let mut opening = Box::pin(crate::store::failed_open_close::scope(
                Arc::clone(&pause),
                open_owner_store(options, hooks),
            ));
            tokio::select! {
                biased;
                () = pause.entered.notified() => {}
                opened = opening.as_mut() => bail!(
                    "the open ended before closing a failed engine: {:?}",
                    opened.map(|_| ())
                ),
            }
            // The close has not begun: the open waits at the pause.
            log.subscribe()
                .wait_for(|writes| {
                    writes
                        .iter()
                        .any(|(_, activity)| activity.failure.is_some())
                })
                .await?;
            let activity = read_activity(&data, &scope, &activity_tag(&token))?;
            let reason = activity
                .failure
                .clone()
                .context("the record is not marked failing")?;
            ensure!(
                reason.contains(crate::store::ACTIVE_VALIDATION_REFUSAL),
                "{reason}"
            );
            ensure!(activity.stages.contains(&OpeningDatabase), "{activity:?}");
            ensure!(activity.progress > 0, "{activity:?}");
            pause.release.notify_one();
            let error = opening
                .await
                .err()
                .context("the open succeeded despite its refused validation")?;
            ensure!(
                format!("{error:#}").contains(crate::store::ACTIVE_VALIDATION_REFUSAL),
                "{error:#}"
            );
            ensure!(
                !directory(&data, &scope)?.join(RECORD).exists(),
                "the failed open left its record"
            );
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| format!("failing mark fixture exceeded its {deadline:?} deadline"))?
    }

    // Two opens in one process: each advances only its own count.
    #[tokio::test]
    async fn concurrent_opens_advance_independent_counts() -> Result<()> {
        warm_runtime_cache().await?;
        let deadline = fixture_deadline(2, 0);
        tokio::time::timeout(deadline, async {
            let (first_root, second_root) = (tempfile::tempdir()?, tempfile::tempdir()?);
            let (_, _, _, first) = owner_fixture(first_root.path())?;
            let (_, _, _, second) = owner_fixture(second_root.path())?;
            let _gate = crate::spawn_gate::spawning().await;
            let (first_progress, first_open) = MemoryStore::open_observed(first);
            let (second_progress, second_open) = MemoryStore::open_observed(second);
            let first_count = first_progress
                .ticks()
                .context("first open counts nothing")?;
            let second_count = second_progress
                .ticks()
                .context("second open counts nothing")?;
            let first_store = first_open.await?;
            let first_total = first_count.borrow().count;
            ensure!(first_total > 0, "the first open never advanced");
            ensure!(
                second_count.borrow().count == 0,
                "the first open advanced the second open's count"
            );
            let second_store = second_open.await?;
            ensure!(second_count.borrow().count > 0);
            ensure!(
                first_count.borrow().count == first_total,
                "the second open advanced the first open's count"
            );
            first_store.close().await?;
            second_store.close().await
        })
        .await
        .with_context(|| format!("concurrent counts fixture exceeded its {deadline:?} deadline"))?
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

    // An owner whose store opened but whose endpoint publication failed marks
    // its record failing with that failure before its store closes, and
    // retires the record after that close, before releasing its owner lock.
    #[tokio::test]
    async fn a_failed_endpoint_publication_marks_then_retires_the_record() -> Result<()> {
        warm_runtime_cache().await?;
        let deadline = fixture_deadline(1, 0);
        tokio::time::timeout(deadline, async {
            let root = tempfile::tempdir()?;
            let (project, scope, data, mut options) = owner_fixture(root.path())?;
            let token = Uuid::new_v4();
            options.starter_token = Some(token);
            let tag = activity_tag(&token);
            // A directory where the endpoint record would be published.
            let services = crate::files::ensure_private_directory(&directory(&data, &scope)?)?;
            std::fs::create_dir(services.path().join("endpoint.json"))?;
            let pause = ClosePause::at(ClosePoint::BeforeStoreClose);
            let hooks = OwnerHooks {
                close_pause: Some(Arc::clone(&pause)),
                ..OwnerHooks::default()
            };
            let _gate = crate::spawn_gate::spawning().await;
            let mut opening = Box::pin({
                let options = options.clone();
                async move { ServiceOwner::open_with_activity(options, &project, hooks).await }
            });
            tokio::select! {
                biased;
                () = pause.entered.notified() => {}
                opened = opening.as_mut() => bail!(
                    "the owner ended before closing its store: {:?}",
                    opened.map(|_| ())
                ),
            }
            // Read at the pause and checked after the release, so a failed
            // check never leaves the owner holding its engine.
            let held = observe(&data, &scope, &tag);
            let lock_free = owner_lock_free(&options);
            pause.release.notify_one();
            let error = opening
                .await
                .err()
                .context("the owner opened despite its occupied endpoint name")?;
            let Observation::Record(activity) = held else {
                bail!("before the store closed the record was {held:?}, not marked failing");
            };
            let reason = activity
                .failure
                .clone()
                .context("the record is not marked failing")?;
            ensure!(reason.contains("endpoint.json"), "{reason}");
            ensure!(
                format!("{error:#}").starts_with(reason.as_str()),
                "the reason {reason:?} is not the returned failure {error:#}"
            );
            for secret in [
                token.hyphenated().to_string(),
                token.simple().to_string(),
                tag.clone(),
            ] {
                ensure!(!reason.contains(&secret), "{reason}");
            }
            ensure!(activity.stages.contains(&OpeningDatabase), "{activity:?}");
            ensure!(activity.progress > 0, "{activity:?}");
            ensure!(
                !lock_free?,
                "the owner lock was free before the store closed"
            );
            ensure!(
                !directory(&data, &scope)?.join(RECORD).exists(),
                "the record outlived the owner's ending"
            );
            ensure!(owner_lock_free(&options)?);
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| {
            format!("failed publication mark fixture exceeded its {deadline:?} deadline")
        })?
    }

    // An owner whose serve loop fails before its starter attached marks its
    // record failing, keeping its final activity, before its store closes,
    // and retires the record after that close.
    #[tokio::test]
    async fn a_serve_error_before_the_starter_marks_then_retires_the_record() -> Result<()> {
        const FAULT: &str = "test listener fault";
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
                .ended()
                .await;
            let Observation::Record(published) = observe(&data, &scope, &tag) else {
                bail!("the opened owner's record is not readable");
            };
            let pause = ClosePause::at(ClosePoint::BeforeStoreClose);
            let (mut knobs, _events) = observed(Admission::Starter(token), None);
            knobs.accept_fault = Some(FAULT);
            knobs.close_pause = Some(Arc::clone(&pause));
            let served = tokio::spawn(owner.serve_with(knobs));
            pause.entered.notified().await;
            // Read at the pause and checked after the release.
            let held = observe(&data, &scope, &tag);
            let endpoint = EndpointRecord::read(&data, &scope).map(|record| record.is_none());
            let lock_free = owner_lock_free(&options);
            pause.release.notify_one();
            let error = served
                .await?
                .err()
                .context("the owner served despite its listener fault")?;
            ensure!(format!("{error:#}") == FAULT, "{error:#}");
            let Observation::Record(activity) = held else {
                bail!("before the store closed the record was {held:?}, not marked failing");
            };
            ensure!(
                activity
                    == Activity {
                        failure: Some(FAULT.to_owned()),
                        ..published.clone()
                    },
                "{activity:?} is not {published:?} marked failing"
            );
            ensure!(endpoint?, "the endpoint outlived the owner's ending");
            ensure!(
                !lock_free?,
                "the owner lock was free before the store closed"
            );
            ensure!(
                !directory(&data, &scope)?.join(RECORD).exists(),
                "the record outlived the owner's ending"
            );
            ensure!(owner_lock_free(&options)?);
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| format!("serve error mark fixture exceeded its {deadline:?} deadline"))?
    }

    // The invariant a retired record's report rests on: a healthy owner keeps
    // its record, unmarked, from endpoint publication until its starter has
    // attached and its endpoint is retired, and retires it before its store
    // closes.
    #[tokio::test]
    async fn a_healthy_owner_keeps_its_record_until_its_starter_attaches() -> Result<()> {
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
                .ended()
                .await;
            let Observation::Record(published) = observe(&data, &scope, &tag) else {
                bail!("the opened owner's record is not readable");
            };
            ensure!(published.failure.is_none(), "{published:?}");
            ensure!(published.progress > 0, "{published:?}");
            let pause =
                ClosePause::at_each(&[ClosePoint::AfterEndpointRetire, ClosePoint::AfterReap]);
            let (mut knobs, mut events) = observed(Admission::Starter(token), None);
            knobs.close_pause = Some(Arc::clone(&pause));
            let served = tokio::spawn(owner.serve_with(knobs));
            // In its loop, endpoint published, starter not attached.
            let entered = next_event(&mut events).await?;
            let waiting = observe(&data, &scope, &tag);
            drop(attach_raw(&data, &scope, Some(token)).await?);
            pause.entered.notified().await;
            let endpoint = EndpointRecord::read(&data, &scope).map(|record| record.is_none());
            let retiring = observe(&data, &scope, &tag);
            pause.release.notify_one();
            pause.entered.notified().await;
            let reaped = directory(&data, &scope).map(|directory| directory.join(RECORD).exists());
            let lock_free = owner_lock_free(&options);
            pause.release.notify_one();
            served.await??;
            ensure!(
                entered == ServeEvent::EnteredEmpty { reached: false },
                "{entered:?}"
            );
            ensure!(
                waiting == Observation::Record(published.clone()),
                "before its starter attached the record was {waiting:?}"
            );
            ensure!(endpoint?, "the endpoint was not retired");
            ensure!(
                retiring == Observation::Record(published),
                "after the endpoint retired the record was {retiring:?}"
            );
            ensure!(!reaped?, "the record outlived the store close");
            ensure!(
                !lock_free?,
                "the owner lock was released before the reap ended"
            );
            ensure!(owner_lock_free(&options)?);
            Ok::<(), anyhow::Error>(())
        })
        .await
        .with_context(|| format!("healthy record fixture exceeded its {deadline:?} deadline"))?
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
                &staged(&[UpgradingDatabase]),
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
