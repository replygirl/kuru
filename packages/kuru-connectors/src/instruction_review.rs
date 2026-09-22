//! Actor-only nested instruction review. This channel is separate from tool
//! permission grants: a file permission cannot approve prompt authority.

use anyhow::Result;
use async_trait::async_trait;
use kuru_core::ProjectRelativeTarget;
use tokio::sync::{mpsc, oneshot};

#[derive(Clone)]
pub struct InstructionReviewSender {
    sender: mpsc::Sender<InstructionReviewRequest>,
}

impl InstructionReviewSender {
    pub fn new(sender: mpsc::Sender<InstructionReviewRequest>) -> Self {
        Self { sender }
    }

    /// A closed or busy foreground channel requires a headless-style remedy.
    pub async fn ask(
        &self,
        display: String,
        persistent_allowed: bool,
    ) -> Option<InstructionReviewAnswer> {
        let (reply, receive) = oneshot::channel();
        self.sender
            .try_send(InstructionReviewRequest {
                display,
                persistent_allowed,
                reply,
            })
            .ok()?;
        receive.await.ok()
    }
}

pub struct InstructionReviewRequest {
    pub display: String,
    pub persistent_allowed: bool,
    pub reply: oneshot::Sender<InstructionReviewAnswer>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstructionReviewAnswer {
    Once,
    Persist,
    Deny,
}

pub enum InstructionGateOutcome {
    Unchanged,
    Proposed(Box<dyn InstructionActivation>),
    Denied,
    Required(String),
}

/// Publication happens only after ToolHost revalidates the exact target once
/// more after any foreground wait. It may persist the reviewed trust choice and
/// install the captured snapshot, then returns the prompt projection.
#[async_trait]
pub trait InstructionActivation: Send {
    async fn publish(self: Box<Self>) -> Result<String>;
}

/// The application owns checked capture and workspace approval storage. The
/// connector supplies only directories derived from already authorized and
/// checked native targets. A file contributes its containing directory; a
/// directory listing contributes the listed directory itself.
#[async_trait]
pub trait InstructionGate: Send + Sync {
    async fn review(
        &self,
        targets: &[ProjectRelativeTarget],
        approval: Option<&InstructionReviewSender>,
    ) -> Result<InstructionGateOutcome>;
}
