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
}

impl MemoryOpenStage {
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
        }
    }
}

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
        let (sender, receiver) = mpsc::channel(16);
        (
            MemoryOpenProgress { receiver },
            Self {
                sender: Some(sender),
                sent: 0,
            },
        )
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
            // The fixed finite stage list fits the channel. A closed receiver
            // or any unexpected full queue only disables decoration.
            self.sender = None;
        }
    }
}
