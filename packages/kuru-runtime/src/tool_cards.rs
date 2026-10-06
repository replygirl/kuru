//! Bounded current-Harness tool presentation, separate from durable events.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use kuru_connectors::{
    ShellProgress, ToolInvocationContext, project_json, project_text, truncate_tool_output,
};
use kuru_core::ToolCall;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::sync::broadcast;

use crate::ToolOutcome;

pub const MAX_TOOL_CARDS: usize = 64;
pub const MAX_TOOL_CARD_BYTES: usize = 1024 * 1024;
// Both stream slots, a terminal snapshot while its pending view is replaced,
// the expanded checked diff and fixed handle metadata count toward the window.
const PREVIEW_RESERVE_BYTES: usize = 4 * kuru_connectors::MAX_SHELL_PREVIEW_BYTES + 8192 + 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ToolCardState {
    Pending,
    Settled(ToolOutcome),
    /// The containing operation ended or recovered without an exact tool
    /// outcome. This never asserts success, cancellation or absence of effects.
    Interrupted,
}

/// Transient presentation only. This is never stored in Event, a turn journal,
/// public transcript, export or TurnOutput.
#[derive(Clone, Serialize)]
pub struct ToolCard {
    pub id: String,
    pub session_id: String,
    pub source_view: String,
    pub turn_id: String,
    pub turn_key: String,
    pub actor_id: String,
    pub call_id: String,
    pub name: String,
    pub ordinal: u64,
    pub state: ToolCardState,
    pub arguments: Option<String>,
    pub output: Option<String>,
    pub private_content: bool,
    pub checkpoint_id: Option<String>,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    pub preview_truncated: bool,
    pub preview_omitted: u64,
    #[serde(skip)]
    pub progress: Option<ShellProgress>,
}

impl std::fmt::Debug for ToolCard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolCard")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub enum ToolCardUpdate {
    Cleared,
    Changed,
}

#[derive(Clone)]
pub(crate) struct ToolCardBinding {
    pub id: String,
    pub progress: Option<ShellProgress>,
}

struct CardWindow {
    cards: VecDeque<Arc<ToolCard>>,
    bytes: usize,
    omitted: u64,
    ordinal: u64,
    owner: Option<(String, String)>,
    updates: broadcast::Sender<ToolCardUpdate>,
}

fn display(value: &str, limit: usize) -> String {
    truncate_tool_output(
        &project_text(value).unwrap_or_else(|_| "[detail withheld]".into()),
        limit,
    )
}

fn retained_bytes(card: &ToolCard) -> usize {
    serde_json::to_vec(card).map_or(MAX_TOOL_CARD_BYTES, |bytes| bytes.len())
        + PREVIEW_RESERVE_BYTES
}

fn turn_key(session: &str, turn: &str, view: &str) -> String {
    let mut hash = Sha256::new();
    for field in ["kuru-tool-presentation-turn-v1", session, turn, view] {
        hash.update((field.len() as u64).to_be_bytes());
        hash.update(field.as_bytes());
    }
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn freeze_preview(card: &mut ToolCard) {
    if let Some(progress) = card.progress.take() {
        let preview = progress.snapshot();
        card.preview_omitted = preview.omitted;
        card.preview_truncated = preview.stdout.as_ref().is_some_and(|value| value.truncated)
            || preview.stderr.as_ref().is_some_and(|value| value.truncated);
        card.stdout = preview.stdout.map(|value| value.text);
        card.stderr = preview.stderr.map(|value| value.text);
    }
}

impl CardWindow {
    pub(crate) fn new() -> Self {
        Self {
            cards: VecDeque::new(),
            bytes: 0,
            omitted: 0,
            ordinal: 0,
            owner: None,
            updates: broadcast::channel(MAX_TOOL_CARDS * 2).0,
        }
    }

    pub(crate) fn clear(&mut self) {
        self.cards.clear();
        self.bytes = 0;
        self.omitted = 0;
        self.owner = None;
        let _ = self.updates.send(ToolCardUpdate::Cleared);
    }

    pub(crate) fn begin(&mut self, session: &str, view: &str) {
        if self
            .owner
            .as_ref()
            .is_some_and(|owner| owner.0 != session || owner.1 != view)
        {
            self.clear();
        }
        self.owner = Some((session.into(), view.into()));
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<ToolCardUpdate> {
        self.updates.subscribe()
    }

    pub(crate) fn snapshot(&self) -> (Vec<Arc<ToolCard>>, u64) {
        (self.cards.iter().cloned().collect(), self.omitted)
    }

    fn prepare(
        owner: &(String, String),
        context: &ToolInvocationContext,
        call: &ToolCall,
        private: bool,
    ) -> Option<(ToolCard, Sha256)> {
        let (session, view) = owner;
        if session != &context.session_id {
            return None;
        }
        let mut hash = Sha256::new();
        for field in [
            "kuru-tool-presentation-v1",
            view,
            &context.session_id,
            &context.turn_id,
            &context.actor_id,
            &context.invocation_id,
            &context.call_id,
        ] {
            hash.update((field.len() as u64).to_be_bytes());
            hash.update(field.as_bytes());
        }
        let progress = (!private && call.name == "shell").then(ShellProgress::new);
        let arguments = if private {
            None
        } else {
            project_json(call.arguments.clone())
                .ok()
                .map(|value| truncate_tool_output(&value.to_string(), 1024))
        };
        let card = ToolCard {
            id: String::new(),
            session_id: display(&context.session_id, 256),
            source_view: display(view, 64),
            turn_id: display(&context.turn_id, 256),
            turn_key: turn_key(&context.session_id, &context.turn_id, view),
            actor_id: display(&context.actor_id, 256),
            call_id: display(&context.call_id, 256),
            name: display(&call.name, 256),
            ordinal: 0,
            state: ToolCardState::Pending,
            arguments,
            output: None,
            private_content: private,
            checkpoint_id: (!private
                && matches!(
                    call.name.as_str(),
                    "file_write" | "file_edit" | "file_delete"
                ))
            .then(|| context.file_checkpoint_id().ok())
            .flatten(),
            stdout: None,
            stderr: None,
            preview_truncated: false,
            preview_omitted: 0,
            progress: progress.clone(),
        };
        Some((card, hash))
    }

    fn admit(
        &mut self,
        owner: &(String, String),
        prepared: (ToolCard, Sha256),
    ) -> Option<ToolCardBinding> {
        if self.owner.as_ref() != Some(owner) {
            return None;
        }
        let (mut card, mut hash) = prepared;
        self.ordinal = self.ordinal.saturating_add(1);
        hash.update(self.ordinal.to_be_bytes());
        card.id = hash
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        card.ordinal = self.ordinal;
        let binding = ToolCardBinding {
            id: card.id.clone(),
            progress: card.progress.clone(),
        };
        let card = Arc::new(card);
        self.cards.push_back(card.clone());
        self.bytes += retained_bytes(&card);
        self.trim();
        let _ = self.updates.send(ToolCardUpdate::Changed);
        Some(binding)
    }

    pub(crate) fn settle(
        &mut self,
        binding: Option<&ToolCardBinding>,
        outcome: ToolOutcome,
        output: Option<String>,
    ) {
        let Some(binding) = binding else {
            return;
        };
        let Some(index) = self.cards.iter().position(|card| card.id == binding.id) else {
            return;
        };
        if self.cards[index].state != ToolCardState::Pending {
            return;
        }
        let mut card = (*self.cards[index]).clone();
        self.bytes -= retained_bytes(&card);
        card.state = ToolCardState::Settled(outcome);
        if !card.private_content {
            card.output = output.map(|text| truncate_tool_output(&text, 8192));
            freeze_preview(&mut card);
        }
        let card = Arc::new(card);
        self.bytes += retained_bytes(&card);
        self.cards[index] = card.clone();
        self.trim();
        let _ = self.updates.send(ToolCardUpdate::Changed);
    }

    fn end_turn(&mut self, session: &str, turn: &str) {
        let Some((owner, view)) = &self.owner else {
            return;
        };
        if owner != session {
            return;
        }
        let key = turn_key(session, turn, view);
        let mut changed = false;
        for retained in &mut self.cards {
            if retained.turn_key == key && retained.state == ToolCardState::Pending {
                self.bytes -= retained_bytes(retained);
                let mut card = (**retained).clone();
                card.state = ToolCardState::Interrupted;
                freeze_preview(&mut card);
                self.bytes += retained_bytes(&card);
                *retained = Arc::new(card);
                changed = true;
            }
        }
        if changed {
            self.trim();
            let _ = self.updates.send(ToolCardUpdate::Changed);
        }
    }

    fn trim(&mut self) {
        let before = self.omitted;
        while self.cards.len() > MAX_TOOL_CARDS || self.bytes > MAX_TOOL_CARD_BYTES {
            let Some(card) = self.cards.pop_front() else {
                break;
            };
            self.bytes -= retained_bytes(&card);
            self.omitted = self.omitted.saturating_add(1);
        }
        if self.omitted != before {
            let _ = self.updates.send(ToolCardUpdate::Changed);
        }
    }
}

/// Read-only transient feed. Notifications contain no bodies or retained handles;
/// lagged consumers can obtain the same bounded current window without replay.
#[derive(Clone)]
pub struct ToolCardFeed(Arc<Mutex<CardWindow>>);

impl ToolCardFeed {
    /// Filter using the exact retained owner, never projected display IDs.
    pub fn snapshot_for_session(&self, session: &str) -> (Vec<Arc<ToolCard>>, u64) {
        let window = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if window
            .owner
            .as_ref()
            .is_some_and(|owner| owner.0 == session)
        {
            window.snapshot()
        } else {
            (Vec::new(), 0)
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ToolCardUpdate> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .subscribe()
    }

    pub fn snapshot(&self) -> (Vec<Arc<ToolCard>>, u64) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .snapshot()
    }
}

pub(crate) struct ToolCards(ToolCardFeed);

impl ToolCards {
    pub(crate) fn new() -> Self {
        Self(ToolCardFeed(Arc::new(Mutex::new(CardWindow::new()))))
    }
    pub(crate) fn feed(&self) -> ToolCardFeed {
        self.0.clone()
    }
    pub(crate) fn clear(&mut self) {
        self.0
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }
    pub(crate) fn begin(&mut self, session: &str, view: &str) {
        self.0
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .begin(session, view);
    }
    pub(crate) fn admit(
        &mut self,
        context: &ToolInvocationContext,
        call: &ToolCall,
        private: bool,
    ) -> Option<ToolCardBinding> {
        let owner = self
            .0
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .owner
            .clone()?;
        let prepared = CardWindow::prepare(&owner, context, call, private)?;
        self.0
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .admit(&owner, prepared)
    }

    pub(crate) fn end_turn(&mut self, session: &str, turn: &str) {
        self.0
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .end_turn(session, turn);
    }

    pub(crate) fn settle(
        &mut self,
        binding: Option<&ToolCardBinding>,
        outcome: ToolOutcome,
        output: Option<String>,
    ) {
        self.0
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .settle(binding, outcome, output);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn context(turn: &str, invocation: &str) -> ToolInvocationContext {
        ToolInvocationContext {
            session_id: "session".into(),
            turn_id: turn.into(),
            actor_id: "actor".into(),
            invocation_id: invocation.into(),
            call_id: "reused-call".into(),
        }
    }

    fn call(name: &str) -> ToolCall {
        ToolCall {
            id: "reused-call".into(),
            name: name.into(),
            arguments: json!({"path":"checked.txt", "content":"sk-abcdefghijklmnop"}),
        }
    }

    #[test]
    fn tool_cards_end_only_the_exact_interrupted_turn_without_inventing_an_outcome() {
        let mut cards = ToolCards::new();
        cards.begin("session", "main");
        let old = cards
            .admit(&context("turn-one", "invocation"), &call("shell"), false)
            .unwrap();
        let new = cards
            .admit(&context("turn-two", "invocation"), &call("shell"), false)
            .unwrap();
        cards.end_turn("session", "turn-one");
        let snapshot = cards.feed().snapshot().0;
        assert_eq!(snapshot[0].state, ToolCardState::Interrupted);
        assert!(snapshot[0].output.is_none() && snapshot[0].progress.is_none());
        assert_eq!(snapshot[1].state, ToolCardState::Pending);
        cards.settle(Some(&old), ToolOutcome::Ok, Some("late old result".into()));
        cards.settle(Some(&new), ToolOutcome::Cancelled, None);
        let snapshot = cards.feed().snapshot().0;
        assert_eq!(snapshot[0].state, ToolCardState::Interrupted);
        assert_eq!(
            snapshot[1].state,
            ToolCardState::Settled(ToolOutcome::Cancelled)
        );
        assert!(snapshot[0].output.is_none());
    }

    #[test]
    fn tool_cards_keep_exact_admission_order_and_reverse_settlement_identity() {
        let mut cards = ToolCards::new();
        cards.begin("session", "main");
        let first = cards
            .admit(
                &context("turn", "invocation-one"),
                &call("file_read"),
                false,
            )
            .unwrap();
        let second = cards
            .admit(
                &context("turn", "invocation-two"),
                &call("file_read"),
                false,
            )
            .unwrap();
        assert_ne!(first.id, second.id);
        cards.settle(Some(&second), ToolOutcome::Denied, None);
        cards.settle(Some(&first), ToolOutcome::Ok, Some("checked output".into()));
        let (snapshot, omitted) = cards.feed().snapshot();
        assert_eq!(omitted, 0);
        assert!(snapshot[0].ordinal < snapshot[1].ordinal);
        assert_eq!(snapshot[0].state, ToolCardState::Settled(ToolOutcome::Ok));
        assert_eq!(
            snapshot[1].state,
            ToolCardState::Settled(ToolOutcome::Denied)
        );
        assert_eq!(snapshot[0].output.as_deref(), Some("checked output"));
        assert!(snapshot.iter().all(|card| {
            !card
                .arguments
                .as_deref()
                .unwrap()
                .contains("sk-abcdefghijklmnop")
        }));
        cards.settle(Some(&first), ToolOutcome::Cancelled, Some("late".into()));
        assert_eq!(
            cards.feed().snapshot().0[0].output.as_deref(),
            Some("checked output")
        );
        let other_turn = cards
            .admit(
                &context("other-turn", "invocation-one"),
                &call("file_read"),
                false,
            )
            .unwrap();
        assert_ne!(first.id, other_turn.id);
    }

    #[test]
    fn tool_cards_withhold_cognitive_arguments_results_and_preview_handles() {
        let mut cards = ToolCards::new();
        cards.begin("session", "main");
        for name in [
            "peer_send",
            "relate",
            "state_report",
            "remember",
            "a2a_send",
        ] {
            let binding = cards
                .admit(&context("turn", name), &call(name), true)
                .unwrap();
            assert!(binding.progress.is_none());
            cards.settle(
                Some(&binding),
                ToolOutcome::Ok,
                Some("PRIVATE_RAW_HISTORY_SENTINEL".into()),
            );
        }
        let encoded = serde_json::to_string(
            &cards
                .feed()
                .snapshot()
                .0
                .iter()
                .map(|card| card.as_ref())
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert!(
            !encoded.contains("PRIVATE_RAW_HISTORY_SENTINEL")
                && !encoded.contains("sk-abcdefghijklmnop")
        );
        assert!(
            cards
                .feed()
                .snapshot()
                .0
                .iter()
                .all(|card| card.private_content
                    && card.arguments.is_none()
                    && card.output.is_none()
                    && card.progress.is_none())
        );
    }

    #[test]
    fn tool_cards_bound_all_fields_and_handles_and_do_not_recreate_evicted_calls() {
        let mut cards = ToolCards::new();
        cards.begin("session", "main");
        let earliest = cards
            .admit(&context("turn", "first"), &call("shell"), false)
            .unwrap();
        for ordinal in 0..200 {
            let binding = cards
                .admit(
                    &context("turn", &format!("invocation-{ordinal}")),
                    &call("shell"),
                    false,
                )
                .unwrap();
            cards.settle(Some(&binding), ToolOutcome::Ok, Some("\u{1}".repeat(8192)));
        }
        let (snapshot, omitted) = cards.feed().snapshot();
        assert!(omitted > 0 && snapshot.len() <= MAX_TOOL_CARDS);
        assert!(
            snapshot
                .iter()
                .map(|card| retained_bytes(card))
                .sum::<usize>()
                <= MAX_TOOL_CARD_BYTES
        );
        assert!(!snapshot.iter().any(|card| card.id == earliest.id));
        cards.settle(
            Some(&earliest),
            ToolOutcome::Ok,
            Some("late evicted result".into()),
        );
        assert!(
            !cards
                .feed()
                .snapshot()
                .0
                .iter()
                .any(|card| card.id == earliest.id)
        );
    }

    #[test]
    fn tool_cards_clear_exact_view_session_ownership_without_accepting_foreign_or_late_cards() {
        let mut cards = ToolCards::new();
        cards.begin("session", "candidate-one");
        let old = cards
            .admit(&context("turn", "invocation"), &call("shell"), false)
            .unwrap();
        cards.begin("session", "main");
        cards.settle(
            Some(&old),
            ToolOutcome::Ok,
            Some("foreign candidate result".into()),
        );
        assert!(cards.feed().snapshot().0.is_empty());
        let current = cards
            .admit(&context("turn", "invocation"), &call("file_write"), false)
            .unwrap();
        assert_ne!(old.id, current.id);
        assert_eq!(
            cards.feed().snapshot().0[0].checkpoint_id.as_deref(),
            context("turn", "invocation")
                .file_checkpoint_id()
                .ok()
                .as_deref()
        );
        cards.begin("another-session", "main");
        assert!(
            cards
                .admit(&context("turn", "invocation"), &call("file_read"), false)
                .is_none()
        );
        cards.settle(
            Some(&current),
            ToolOutcome::Ok,
            Some("old session output".into()),
        );
        assert!(cards.feed().snapshot().0.is_empty());
    }

    #[test]
    fn tool_cards_filter_exact_session_without_comparing_projected_display_text() {
        let mut cards = ToolCards::new();
        let session = format!("sk-proj-abcdefghijklmnop{}", "long".repeat(100));
        cards.begin(&session, "main");
        let mut invocation = context("turn", "invocation");
        invocation.session_id = session.clone();
        cards.admit(&invocation, &call("shell"), false).unwrap();
        let feed = cards.feed();
        let selected = feed.snapshot_for_session(&session).0;
        assert_eq!(selected.len(), 1);
        assert_ne!(selected[0].session_id, session);
        assert!(
            feed.snapshot_for_session(&selected[0].session_id)
                .0
                .is_empty()
        );
        assert!(feed.snapshot_for_session("another-session").0.is_empty());
    }
}
