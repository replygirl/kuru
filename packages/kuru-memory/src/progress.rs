use std::sync::Arc;

use tokio::sync::{mpsc, watch};

use crate::open_timeline;

/// A coarse observation of one actual memory-open operation.
///
/// Stages report that Kuru has begun the named work. They carry no path,
/// duration, byte count, credential, query, or failure detail; the eventual
/// open result remains authoritative.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryOpenStage {
    WaitingForProjectOwnership,
    WaitingForRuntimeCache,
    VerifyingRuntimeCache,
    ExtractingEmbeddedRuntime,
    CheckingRuntimeVersion,
    PreparingDatabase,
    OpeningDatabase,
    Ready,
    /// A published, verified engine kept its private install stage after its
    /// own bounded removal did not complete; a receipt was written for a
    /// later collection. This never changes the open's success.
    RetainedInstallStage,
    /// The same retention, except that the receipt itself could not be
    /// written. No later sweep can find this stage from disk, so it is
    /// reported separately: nothing here promises a later collection.
    RetainedUnreceiptedInstallStage,
    /// This process is about to create the project's first database.
    CreatingDatabase,
    /// This process is about to upgrade the project's existing database to
    /// the current schema.
    UpgradingDatabase,
    /// A client has finished waiting for a previous owner and is starting
    /// the project's memory service. It ends that wait for a reader, since
    /// a stage is never reported twice.
    StartingMemoryService,
}

/// The observation channel's capacity. Every stage is reported at most once
/// per reporter, so the whole finite list fits without a reader keeping up.
const CHANNEL_CAPACITY: usize = 16;

impl MemoryOpenStage {
    /// Every stage, in declaration order. The assertion below keeps this list
    /// within the channel's capacity and the `sent` mask, so adding a stage
    /// past either bound fails to compile instead of dropping observations.
    pub(crate) const ALL: [Self; 13] = [
        Self::WaitingForProjectOwnership,
        Self::WaitingForRuntimeCache,
        Self::VerifyingRuntimeCache,
        Self::ExtractingEmbeddedRuntime,
        Self::CheckingRuntimeVersion,
        Self::PreparingDatabase,
        Self::OpeningDatabase,
        Self::Ready,
        Self::RetainedInstallStage,
        Self::RetainedUnreceiptedInstallStage,
        Self::CreatingDatabase,
        Self::UpgradingDatabase,
        Self::StartingMemoryService,
    ];

    const fn bit(self) -> u16 {
        match self {
            Self::WaitingForProjectOwnership => 1 << 0,
            Self::WaitingForRuntimeCache => 1 << 1,
            Self::VerifyingRuntimeCache => 1 << 2,
            Self::ExtractingEmbeddedRuntime => 1 << 3,
            Self::CheckingRuntimeVersion => 1 << 4,
            Self::PreparingDatabase => 1 << 5,
            Self::OpeningDatabase => 1 << 6,
            Self::Ready => 1 << 7,
            Self::RetainedInstallStage => 1 << 8,
            Self::RetainedUnreceiptedInstallStage => 1 << 9,
            Self::CreatingDatabase => 1 << 10,
            Self::UpgradingDatabase => 1 << 11,
            Self::StartingMemoryService => 1 << 12,
        }
    }
}

const _: () = {
    assert!(MemoryOpenStage::ALL.len() <= CHANNEL_CAPACITY);
    assert!(MemoryOpenStage::ALL.len() <= u16::BITS as usize);
    // Each listed stage owns one distinct bit, with none skipped.
    let mut seen = 0_u16;
    let mut index = 0;
    while index < MemoryOpenStage::ALL.len() {
        let bit = MemoryOpenStage::ALL[index].bit();
        assert!(seen & bit == 0);
        seen |= bit;
        index += 1;
    }
    assert!(seen == u16::MAX >> (u16::BITS as usize - MemoryOpenStage::ALL.len()));
};

/// Bounded observations from an explicitly observed memory open.
///
/// Dropping this receiver only disables observations. It does not own or
/// cancel a store, lock, directory, server, migration, extraction, or probe.
#[derive(Debug)]
pub struct MemoryOpenProgress {
    receiver: mpsc::Receiver<MemoryOpenStage>,
    /// The open's own progress count and failing mark, when it counts.
    ticks: Option<watch::Receiver<OpenAdvance>>,
}

impl MemoryOpenProgress {
    pub async fn recv(&mut self) -> Option<MemoryOpenStage> {
        self.receiver.recv().await
    }

    /// The progress count and failing mark of this one open, for its owner's
    /// activity record. Grants nothing.
    pub(crate) fn ticks(&self) -> Option<watch::Receiver<OpenAdvance>> {
        self.ticks.clone()
    }
}

/// Where one open's own work stands: a count that advances only when the
/// open reaches a distinct one-shot point or completes a bounded unit of
/// work, and the failure reason once an open that will fail is about to
/// close the engine it started.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct OpenAdvance {
    pub(crate) count: u64,
    pub(crate) failure: Option<Arc<str>>,
}

/// The counter of one open, shared by its reporter, its engine starts and
/// its workers. Advancing is synchronous and does no I/O, so a blocking
/// thread may advance it; nothing waits for a reader. Never advance it on a
/// timer, inside a sleep, a retry iteration or a wait for a lock.
#[derive(Clone)]
pub struct OpenTicks(Arc<watch::Sender<OpenAdvance>>);

impl std::fmt::Debug for OpenTicks {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("OpenTicks")
    }
}

impl OpenTicks {
    pub(crate) fn new() -> (Self, watch::Receiver<OpenAdvance>) {
        let (sender, receiver) = watch::channel(OpenAdvance::default());
        (Self(Arc::new(sender)), receiver)
    }

    pub(crate) fn advance(&self) {
        self.0
            .send_modify(|advance| advance.count = advance.count.saturating_add(1));
    }

    /// Mark the open failing with `error`'s own text, immediately before it
    /// closes the engine it started and returns that error. The first mark
    /// stands: a later failure in the same close is not the cause.
    pub(crate) fn mark_failing(&self, error: &anyhow::Error) {
        let reason: Arc<str> = Arc::from(format!("{error:#}"));
        self.0.send_if_modified(|advance| {
            if advance.failure.is_some() {
                return false;
            }
            advance.failure = Some(reason);
            true
        });
    }
}

/// Stamp `event` on the open timeline and advance the open's count: every
/// milestone stamped inside an open is also a progress point.
pub(crate) fn milestone(ticks: Option<&OpenTicks>, event: open_timeline::Event) {
    open_timeline::stamp(event);
    if let Some(ticks) = ticks {
        ticks.advance();
    }
}

/// Bytes per progress advance while extracting or hashing the engine.
pub(crate) const BYTES_PER_ADVANCE: u64 = 8 * 1024 * 1024;

/// Advances an open's count once per [`BYTES_PER_ADVANCE`] bytes of one
/// bounded copy or hash, never for a partial chunk.
pub(crate) struct ByteTicks<'a> {
    ticks: Option<&'a OpenTicks>,
    pending: u64,
}

impl<'a> ByteTicks<'a> {
    pub(crate) fn new(ticks: Option<&'a OpenTicks>) -> Self {
        Self { ticks, pending: 0 }
    }

    pub(crate) fn add(&mut self, bytes: usize) {
        let Some(ticks) = self.ticks else {
            return;
        };
        self.pending = self.pending.saturating_add(bytes as u64);
        while self.pending >= BYTES_PER_ADVANCE {
            self.pending -= BYTES_PER_ADVANCE;
            ticks.advance();
        }
    }
}

/// A writer that counts what it forwards into one [`ByteTicks`], which may
/// span several files of one extraction.
pub(crate) struct TickingWriter<'a, 'b, W> {
    inner: W,
    bytes: &'a mut ByteTicks<'b>,
}

impl<'a, 'b, W: std::io::Write> TickingWriter<'a, 'b, W> {
    pub(crate) fn new(inner: W, bytes: &'a mut ByteTicks<'b>) -> Self {
        Self { inner, bytes }
    }
}

impl<W: std::io::Write> std::io::Write for TickingWriter<'_, '_, W> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let written = self.inner.write(buffer)?;
        self.bytes.add(written);
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

pub(crate) struct ProgressReporter {
    sender: Option<mpsc::Sender<MemoryOpenStage>>,
    sent: u16,
    ticks: Option<OpenTicks>,
}

impl ProgressReporter {
    pub(crate) fn silent() -> Self {
        Self {
            sender: None,
            sent: 0,
            ticks: None,
        }
    }

    pub(crate) fn observed() -> (MemoryOpenProgress, Self) {
        let (sender, receiver) = mpsc::channel(CHANNEL_CAPACITY);
        (
            MemoryOpenProgress {
                receiver,
                ticks: None,
            },
            Self {
                sender: Some(sender),
                sent: 0,
                ticks: None,
            },
        )
    }

    /// `observed`, plus one progress counter for this open alone.
    pub(crate) fn observed_counting() -> (MemoryOpenProgress, Self) {
        let (mut progress, mut reporter) = Self::observed();
        let (ticks, receiver) = OpenTicks::new();
        progress.ticks = Some(receiver);
        reporter.ticks = Some(ticks);
        (progress, reporter)
    }

    /// This open's counter, for its engine starts and workers.
    pub(crate) fn ticks(&self) -> Option<&OpenTicks> {
        self.ticks.as_ref()
    }

    /// Stamp `event` and advance this open's count.
    pub(crate) fn milestone(&self, event: open_timeline::Event) {
        milestone(self.ticks(), event);
    }

    /// Whether a reader is still attached, so a caller can skip work that
    /// exists only to feed it.
    pub(crate) fn is_observed(&self) -> bool {
        self.sender
            .as_ref()
            .is_some_and(|sender| !sender.is_closed())
    }

    pub(crate) fn report(&mut self, stage: MemoryOpenStage) {
        // Every report call is a one-shot point of the open, a repeat
        // included, so it advances before the once-per-stage filter.
        if let Some(ticks) = &self.ticks {
            ticks.advance();
        }
        let bit = stage.bit();
        if self.sent & bit != 0 {
            return;
        }
        self.sent |= bit;
        let Some(sender) = self.sender.as_ref() else {
            return;
        };
        if sender.try_send(stage).is_err() {
            // The finite stage list (`MemoryOpenStage::ALL`) fits the channel,
            // and the assertion above keeps it so. A closed receiver or any
            // unexpected full queue only disables decoration.
            self.sender = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read as _;

    const NEW_STAGES: [MemoryOpenStage; 3] = [
        MemoryOpenStage::CreatingDatabase,
        MemoryOpenStage::UpgradingDatabase,
        MemoryOpenStage::StartingMemoryService,
    ];

    fn drain(progress: &mut MemoryOpenProgress) -> Vec<MemoryOpenStage> {
        let mut stages = Vec::new();
        while let Ok(stage) = progress.receiver.try_recv() {
            stages.push(stage);
        }
        stages
    }

    #[test]
    fn every_stage_fits_one_undrained_reporter_in_order() {
        let (mut progress, mut reporter) = ProgressReporter::observed();
        for stage in MemoryOpenStage::ALL {
            reporter.report(stage);
        }
        assert!(reporter.is_observed(), "no stage may overflow the channel");
        assert_eq!(drain(&mut progress), MemoryOpenStage::ALL);
    }

    #[test]
    fn the_new_stages_are_listed_and_keep_their_own_bits() {
        for stage in NEW_STAGES {
            assert!(MemoryOpenStage::ALL.contains(&stage), "{stage:?}");
        }
        let mut seen = 0_u16;
        for stage in MemoryOpenStage::ALL {
            assert_eq!(seen & stage.bit(), 0, "{stage:?} shares a bit");
            seen |= stage.bit();
        }
    }

    #[test]
    fn each_new_stage_is_sent_once_per_reporter() {
        let (mut progress, mut reporter) = ProgressReporter::observed();
        for stage in NEW_STAGES.into_iter().chain(NEW_STAGES) {
            reporter.report(stage);
        }
        assert_eq!(drain(&mut progress), NEW_STAGES);
    }

    #[test]
    fn a_new_stage_does_not_suppress_an_older_one() {
        let (mut progress, mut reporter) = ProgressReporter::observed();
        reporter.report(MemoryOpenStage::StartingMemoryService);
        reporter.report(MemoryOpenStage::WaitingForProjectOwnership);
        reporter.report(MemoryOpenStage::OpeningDatabase);
        assert_eq!(
            drain(&mut progress),
            [
                MemoryOpenStage::StartingMemoryService,
                MemoryOpenStage::WaitingForProjectOwnership,
                MemoryOpenStage::OpeningDatabase,
            ]
        );
    }

    #[test]
    fn every_report_advances_the_count_a_repeat_included() {
        let (mut progress, mut reporter) = ProgressReporter::observed_counting();
        let ticks = progress.ticks().expect("a counting reporter has ticks");
        reporter.report(MemoryOpenStage::OpeningDatabase);
        reporter.report(MemoryOpenStage::OpeningDatabase);
        reporter.report(MemoryOpenStage::Ready);
        assert_eq!(ticks.borrow().count, 3);
        assert_eq!(
            drain(&mut progress),
            [MemoryOpenStage::OpeningDatabase, MemoryOpenStage::Ready]
        );
    }

    #[test]
    fn a_plain_or_silent_reporter_counts_nothing() {
        let (progress, mut reporter) = ProgressReporter::observed();
        assert!(progress.ticks().is_none() && reporter.ticks().is_none());
        reporter.report(MemoryOpenStage::OpeningDatabase);
        let mut silent = ProgressReporter::silent();
        silent.report(MemoryOpenStage::OpeningDatabase);
        silent.milestone(open_timeline::Event::MainPool);
        assert!(silent.ticks().is_none());
    }

    #[test]
    fn two_opens_advance_only_their_own_counts() {
        let (first, mut first_reporter) = ProgressReporter::observed_counting();
        let (second, second_reporter) = ProgressReporter::observed_counting();
        let (first, second) = (first.ticks().unwrap(), second.ticks().unwrap());
        first_reporter.report(MemoryOpenStage::PreparingDatabase);
        first_reporter.milestone(open_timeline::Event::MainPool);
        milestone(first_reporter.ticks(), open_timeline::Event::VersionRead);
        assert_eq!((first.borrow().count, second.borrow().count), (3, 0));
        second_reporter.ticks().unwrap().advance();
        assert_eq!((first.borrow().count, second.borrow().count), (3, 1));
    }

    #[test]
    fn the_first_failing_mark_stands_and_names_its_error() {
        let (ticks, advance) = OpenTicks::new();
        ticks.advance();
        ticks.mark_failing(&anyhow::anyhow!("engine refused").context("open main pool"));
        ticks.mark_failing(&anyhow::anyhow!("close also failed"));
        let advance = advance.borrow();
        assert_eq!(advance.count, 1, "a failing mark is not progress");
        assert_eq!(
            advance.failure.as_deref(),
            Some("open main pool: engine refused")
        );
    }

    #[test]
    fn bytes_advance_once_per_eight_mebibytes_across_writers() {
        use std::io::Write as _;
        let (ticks, advance) = OpenTicks::new();
        let mut written = ByteTicks::new(Some(&ticks));
        // Two files of one extraction share one counter: 5 + 5 MiB is one.
        for _ in 0..2 {
            let mut writer = TickingWriter::new(std::io::sink(), &mut written);
            std::io::copy(&mut std::io::repeat(0).take(5 * 1024 * 1024), &mut writer).unwrap();
            writer.flush().unwrap();
        }
        assert_eq!(advance.borrow().count, 1);
        let mut writer = TickingWriter::new(std::io::sink(), &mut written);
        std::io::copy(
            &mut std::io::repeat(0).take(3 * BYTES_PER_ADVANCE - 2 * 1024 * 1024),
            &mut writer,
        )
        .unwrap();
        // 10 MiB + 22 MiB = 32 MiB: exactly four, never a partial one.
        assert_eq!(advance.borrow().count, 4);
        written.add(BYTES_PER_ADVANCE as usize - 1);
        assert_eq!(advance.borrow().count, 4);
        let mut uncounted = ByteTicks::new(None);
        uncounted.add(usize::MAX);
    }

    #[test]
    fn a_silent_reporter_is_not_observed() {
        let mut reporter = ProgressReporter::silent();
        assert!(!reporter.is_observed());
        reporter.report(MemoryOpenStage::CreatingDatabase);
        assert!(!reporter.is_observed());
    }

    #[test]
    fn an_observed_reporter_stops_being_observed_when_the_receiver_drops() {
        let (progress, mut reporter) = ProgressReporter::observed();
        assert!(reporter.is_observed());
        drop(progress);
        assert!(!reporter.is_observed());
        reporter.report(MemoryOpenStage::Ready);
        assert!(!reporter.is_observed());
    }
}
