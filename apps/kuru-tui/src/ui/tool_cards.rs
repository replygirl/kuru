//! Current-session card state; this never dispatches or reconstructs tool work.

use std::sync::Arc;

use futures::{StreamExt, stream::FuturesUnordered};
use kuru_connectors::{CheckpointDiff, ShellPreviewSnapshot};
use kuru_runtime::{ToolCard, ToolCardState};

use super::View;

#[derive(Debug)]
pub(super) struct CacheIdentity(pub(super) uuid::Uuid);

impl Default for CacheIdentity {
    fn default() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl Clone for CacheIdentity {
    fn clone(&self) -> Self {
        Self::default()
    }
}

#[derive(Clone)]
pub(super) struct DiffResult {
    pub session: String,
    pub card: String,
    pub receipt: String,
    pub diff: CheckpointDiff,
}

#[derive(Debug, Clone)]
pub(super) struct CardView {
    pub card: Arc<ToolCard>,
    pub anchor: Option<super::transcript::ItemId>,
    pub expanded: bool,
    pub stdout: String,
    pub stderr: String,
    pub sequence: u64,
    pub layout_epoch: u64,
    pub gap: bool,
    pub diff: Option<CheckpointDiff>,
}

pub(super) fn safe(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_control() || *character == '\n')
        .collect()
}

impl View {
    pub(super) fn refresh_tool_cards(&mut self, snapshot: (Vec<Arc<ToolCard>>, u64)) {
        let (cards, omitted) = snapshot;
        let mut changed = self.tool_cards_omitted != omitted;
        let next = cards
            .into_iter()
            .filter(|card| card.ordinal > self.tool_cards_hidden_through)
            .map(|card| {
                let anchor = self
                    .transcript
                    .iter()
                    .zip(&self.transcript.items)
                    .find(|((role, _), item)| {
                        role == "user" && item.turn_key.as_deref() == Some(card.turn_key.as_str())
                    })
                    .map(|(_, item)| item.id.clone());
                if let Some(old) = self.tool_cards.iter().find(|old| old.card.id == card.id) {
                    if Arc::ptr_eq(&old.card, &card) && old.anchor == anchor {
                        return old.clone();
                    }
                    changed = true;
                    let mut next = old.clone();
                    next.card = card;
                    next.layout_epoch = next.layout_epoch.wrapping_add(1);
                    next.anchor = anchor;
                    if next.card.state != ToolCardState::Pending {
                        next.stdout = next.card.stdout.as_deref().map(safe).unwrap_or_default();
                        next.stderr = next.card.stderr.as_deref().map(safe).unwrap_or_default();
                        next.gap |= next.card.preview_truncated || next.card.preview_omitted > 0;
                    }
                    next
                } else {
                    changed = true;
                    CardView {
                        stdout: card.stdout.as_deref().map(safe).unwrap_or_default(),
                        stderr: card.stderr.as_deref().map(safe).unwrap_or_default(),
                        gap: card.preview_truncated || card.preview_omitted > 0,
                        card,
                        anchor,
                        expanded: false,
                        sequence: 0,
                        layout_epoch: 0,
                        diff: None,
                    }
                }
            })
            .collect::<Vec<_>>();
        changed |= next.len() != self.tool_cards.len();
        self.tool_cards = next;
        self.tool_cards_omitted = omitted;
        if self
            .selected_tool_card
            .as_ref()
            .is_some_and(|id| !self.tool_cards.iter().any(|card| &card.card.id == id))
        {
            self.selected_tool_card = None;
        }
        if changed {
            self.tool_card_epoch = self.tool_card_epoch.wrapping_add(1);
        }
    }

    pub(super) fn select_tool_card(&mut self) {
        if self.tool_cards.is_empty() {
            self.notify("Tool output is available only for calls observed in this session");
            return;
        }
        let next = self
            .selected_tool_card
            .as_ref()
            .and_then(|id| self.tool_cards.iter().position(|card| &card.card.id == id))
            .map_or(0, |index| (index + 1) % self.tool_cards.len());
        self.selected_tool_card = Some(self.tool_cards[next].card.id.clone());
        self.tool_card_epoch = self.tool_card_epoch.wrapping_add(1);
        self.show_scene = false;
    }

    pub(super) fn toggle_tool_card(&mut self) {
        if self.selected_tool_card.is_none() {
            self.select_tool_card();
        }
        if let Some(card) = self
            .tool_cards
            .iter_mut()
            .find(|card| Some(&card.card.id) == self.selected_tool_card.as_ref())
        {
            card.expanded = !card.expanded;
            card.layout_epoch = card.layout_epoch.wrapping_add(1);
            self.tool_card_epoch = self.tool_card_epoch.wrapping_add(1);
            self.show_scene = false;
        }
    }

    pub(super) fn tool_preview(&mut self, id: &str, preview: ShellPreviewSnapshot) -> bool {
        let Some(card) = self
            .tool_cards
            .iter_mut()
            .find(|card| card.card.id == id && card.card.state == ToolCardState::Pending)
        else {
            return false;
        };
        if preview.sequence <= card.sequence {
            return false;
        }
        card.sequence = preview.sequence;
        card.layout_epoch = card.layout_epoch.wrapping_add(1);
        card.stdout = preview
            .stdout
            .as_ref()
            .map(|preview| safe(&preview.text))
            .unwrap_or_default();
        card.stderr = preview
            .stderr
            .as_ref()
            .map(|preview| safe(&preview.text))
            .unwrap_or_default();
        card.gap = preview.omitted > 0
            || preview.stdout.as_ref().is_some_and(|value| value.truncated)
            || preview.stderr.as_ref().is_some_and(|value| value.truncated);
        self.tool_card_epoch = self.tool_card_epoch.wrapping_add(1);
        true
    }

    pub(super) fn next_diff(&self) -> Option<(String, String)> {
        self.tool_cards.iter().find_map(|card| {
            (card.expanded && card.diff.is_none() && card.card.state != ToolCardState::Pending)
                .then(|| {
                    card.card
                        .checkpoint_id
                        .as_ref()
                        .map(|receipt| (card.card.id.clone(), receipt.clone()))
                })
                .flatten()
        })
    }

    pub(super) fn accept_diff(&mut self, result: DiffResult) -> bool {
        if result.session != self.session {
            return false;
        }
        let Some(card) = self.tool_cards.iter_mut().find(|card| {
            card.card.id == result.card && card.card.checkpoint_id.as_ref() == Some(&result.receipt)
        }) else {
            return false;
        };
        card.diff = Some(result.diff);
        card.layout_epoch = card.layout_epoch.wrapping_add(1);
        self.tool_card_epoch = self.tool_card_epoch.wrapping_add(1);
        true
    }
}

pub(super) async fn next_preview(cards: &[CardView]) -> (String, ShellPreviewSnapshot) {
    let mut pending = FuturesUnordered::new();
    for card in cards
        .iter()
        .filter(|card| card.card.state == ToolCardState::Pending)
    {
        if let Some(progress) = card.card.progress.clone() {
            let id = card.card.id.clone();
            let sequence = card.sequence;
            pending.push(async move { (id, progress.changed_after(sequence).await) });
        }
    }
    match pending.next().await {
        Some(update) => update,
        None => std::future::pending().await,
    }
}
