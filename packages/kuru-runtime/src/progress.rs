//! Ephemeral, replaceable progress for one outward-facing provider request.

use std::sync::{Arc, Mutex};

use anyhow::Result;
use kuru_connectors::{ProviderEvent, ProviderSink};
use kuru_core::{ContextEstimate, ContextSourceSize, UsagePhase};
use tokio::sync::watch;

const TEXT_LIMIT: usize = 8 * 1024;
const SUMMARY_LIMIT: usize = 2 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacingProgress {
    pub turn_id: String,
    pub request_round: u32,
    pub seq: u64,
    pub text_tail: String,
    pub text_truncated: bool,
    pub summary_tail: String,
    pub summary_truncated: bool,
    pub activity: String,
    pub activity_truncated: bool,
}

/// Bounded, replaceable facts for one prepared provider request. No request
/// content or opaque native continuation is retained here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestContext {
    pub operation_id: String,
    pub actor_id: String,
    pub phase: UsagePhase,
    pub estimate: ContextEstimate,
    /// Runtime-visible source inventory; final wire fit remains connector-owned.
    pub runtime_sources: Vec<ContextSourceSize>,
    pub omitted_public_rows: u64,
    pub omitted_private_rows: u64,
    pub omitted_note_rows: u64,
}

/// At most the latest prepared request and the latest facing request are kept.
/// A private consultation or automatic dream cannot erase the facing fit that
/// the just-completed user turn needs to present.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContextSnapshot {
    pub latest: Option<RequestContext>,
    pub latest_facing: Option<RequestContext>,
}

struct Shared {
    sender: watch::Sender<Option<FacingProgress>>,
    active: bool,
    seq: u64,
}

/// Owns the lifetime of previews for an admitted turn. Dropping it fences a
/// still-running actor before clearing the transport slot.
pub(crate) struct ProgressTurn {
    shared: Arc<Mutex<Shared>>,
    turn_id: String,
}

#[derive(Clone)]
pub(crate) struct ProgressDescriptor {
    shared: Arc<Mutex<Shared>>,
    turn_id: String,
    request_round: u32,
}

pub(crate) struct ProgressObserver {
    descriptor: ProgressDescriptor,
    text_tail: String,
    text_truncated: bool,
    summary_tail: String,
    summary_truncated: bool,
    activity: String,
    activity_truncated: bool,
}

impl ProgressTurn {
    pub(crate) fn new(sender: watch::Sender<Option<FacingProgress>>, turn_id: String) -> Self {
        sender.send_replace(None);
        Self {
            shared: Arc::new(Mutex::new(Shared {
                sender,
                active: true,
                seq: 0,
            })),
            turn_id,
        }
    }

    pub(crate) fn round(&self, request_round: u32) -> ProgressDescriptor {
        let descriptor = ProgressDescriptor {
            shared: self.shared.clone(),
            turn_id: self.turn_id.clone(),
            request_round,
        };
        descriptor.publish("", false, "", false, "Responding", false);
        descriptor
    }

    pub(crate) fn finish(&self) {
        let mut shared = self.shared.lock().expect("progress mutex poisoned");
        shared.active = false;
        shared.sender.send_replace(None);
    }
}

impl Drop for ProgressTurn {
    fn drop(&mut self) {
        self.finish();
    }
}

impl ProgressDescriptor {
    pub(crate) fn observer(&self) -> ProgressObserver {
        ProgressObserver {
            descriptor: self.clone(),
            text_tail: String::new(),
            text_truncated: false,
            summary_tail: String::new(),
            summary_truncated: false,
            activity: "Responding".into(),
            activity_truncated: false,
        }
    }

    fn publish(
        &self,
        text_tail: &str,
        text_truncated: bool,
        summary_tail: &str,
        summary_truncated: bool,
        activity: &str,
        activity_truncated: bool,
    ) {
        let mut shared = self.shared.lock().expect("progress mutex poisoned");
        if !shared.active {
            return;
        }
        shared.seq = shared.seq.saturating_add(1);
        shared.sender.send_replace(Some(FacingProgress {
            turn_id: self.turn_id.clone(),
            request_round: self.request_round,
            seq: shared.seq,
            text_tail: text_tail.into(),
            text_truncated,
            summary_tail: summary_tail.into(),
            summary_truncated,
            activity: activity.into(),
            activity_truncated,
        }));
    }
}

impl ProgressObserver {
    pub(crate) fn text(&mut self, fragment: &str) {
        push_tail(
            &mut self.text_tail,
            fragment,
            TEXT_LIMIT,
            &mut self.text_truncated,
        );
        self.publish();
    }

    pub(crate) fn summary(&mut self, fragment: &str) {
        push_tail(
            &mut self.summary_tail,
            fragment,
            SUMMARY_LIMIT,
            &mut self.summary_truncated,
        );
        self.publish();
    }

    fn publish(&self) {
        self.descriptor.publish(
            &self.text_tail,
            self.text_truncated,
            &self.summary_tail,
            self.summary_truncated,
            &self.activity,
            self.activity_truncated,
        );
    }
}

impl ProviderSink for ProgressObserver {
    fn emit<'a>(
        &'a mut self,
        event: ProviderEvent,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            match event {
                ProviderEvent::TextDelta { text, .. } => self.text(&text),
                ProviderEvent::ReasoningSummaryDelta { text, .. } => self.summary(&text),
                ProviderEvent::ToolCallDelta { .. }
                | ProviderEvent::ContextMeasured(_)
                | ProviderEvent::Usage(_)
                | ProviderEvent::Completed(_)
                | ProviderEvent::Failed { .. } => {}
            }
            Ok(())
        })
    }
}

fn push_tail(tail: &mut String, fragment: &str, limit: usize, truncated: &mut bool) {
    tail.push_str(fragment);
    if tail.len() > limit {
        let mut start = tail.len() - limit;
        while !tail.is_char_boundary(start) {
            start += 1;
        }
        tail.drain(..start);
        *truncated = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_tail_is_utf8_bounded_and_terminally_fenced() {
        let (sender, mut receiver) = watch::channel(None);
        let turn = ProgressTurn::new(sender, "turn".into());
        let mut observer = turn.round(1).observer();
        observer.text(&"🙂".repeat(TEXT_LIMIT));
        let preview = receiver.borrow_and_update().clone().unwrap();
        assert_eq!(preview.turn_id, "turn");
        assert_eq!(preview.request_round, 1);
        assert_eq!(preview.text_tail.len(), TEXT_LIMIT);
        assert!(preview.text_truncated);
        drop(turn);
        observer.text("late");
        assert!(receiver.borrow().is_none());
    }
}
