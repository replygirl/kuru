use tokio::sync::mpsc;

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
}

impl MemoryOpenProgress {
    pub async fn recv(&mut self) -> Option<MemoryOpenStage> {
        self.receiver.recv().await
    }
}

pub(crate) struct ProgressReporter {
    sender: Option<mpsc::Sender<MemoryOpenStage>>,
    sent: u16,
}

impl ProgressReporter {
    pub(crate) fn silent() -> Self {
        Self {
            sender: None,
            sent: 0,
        }
    }

    pub(crate) fn observed() -> (MemoryOpenProgress, Self) {
        let (sender, receiver) = mpsc::channel(CHANNEL_CAPACITY);
        (
            MemoryOpenProgress { receiver },
            Self {
                sender: Some(sender),
                sent: 0,
            },
        )
    }

    /// Whether a reader is still attached, so a caller can skip work that
    /// exists only to feed it.
    pub(crate) fn is_observed(&self) -> bool {
        self.sender
            .as_ref()
            .is_some_and(|sender| !sender.is_closed())
    }

    pub(crate) fn report(&mut self, stage: MemoryOpenStage) {
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
