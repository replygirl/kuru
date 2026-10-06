//! Owned public display identities and bounded retained transcript pages.

use std::{collections::VecDeque, ops::Deref};

use kuru_memory::{PublicTranscriptCursor, PublicTranscriptEntry, PublicTranscriptPage};
use sha2::{Digest, Sha256};

use super::project_transcript_message;

pub(super) const PAGE_RECORDS: usize = 32;
const LOCAL_ITEMS: usize = 128;
const LOCAL_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum ItemId {
    Turn { node: String, slot: MessageSlot },
    Legacy(i64),
    Local(uuid::Uuid),
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum MessageSlot {
    User,
    Terminal(usize),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Anchor {
    pub(super) item: ItemId,
    pub(super) row: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) enum Position {
    #[default]
    FollowTail,
    Reading(Anchor),
}

#[derive(Clone, Debug, Default)]
pub(super) struct Navigation {
    pub(super) position: Position,
    pub(super) pending_rows: i64,
    pub(super) match_byte: Option<(ItemId, usize)>,
    pub(super) older_needed: bool,
    pub(super) newer_needed: bool,
    pub(super) newer_scan: Option<Option<PublicTranscriptCursor>>,
    pub(super) newest_needed: bool,
    pub(super) epoch: u64,
    pub(super) restore: Option<Option<PublicTranscriptCursor>>,
}

impl Navigation {
    pub(super) fn scroll(&mut self, rows: i64) {
        self.pending_rows = self.pending_rows.saturating_add(rows);
    }

    pub(super) fn follow(&mut self) {
        self.position = Position::FollowTail;
        self.pending_rows = 0;
        self.match_byte = None;
        self.older_needed = false;
        self.newer_needed = false;
        self.newer_scan = None;
        self.newest_needed = true;
    }

    pub(super) fn reset(&mut self) {
        self.position = Position::FollowTail;
        self.pending_rows = 0;
        self.match_byte = None;
        self.older_needed = false;
        self.newer_needed = false;
        self.newer_scan = None;
        self.newest_needed = false;
        self.restore = None;
        self.epoch = self.epoch.wrapping_add(1);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Item {
    pub(super) id: ItemId,
    /// Replacing this owned text projection changes its layout identity even
    /// when the durable message ID stays the same (for example a hook rewrite).
    pub(super) projection: uuid::Uuid,
    pub(super) turn_key: Option<String>,
    local: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Page {
    pub(super) session: String,
    pub(super) view: String,
    pub(super) revision: String,
    pub(super) head: Option<String>,
    pub(super) total: u64,
    pub(super) next: Option<PublicTranscriptCursor>,
    pub(super) entry: Option<PublicTranscriptCursor>,
    records: usize,
    items: usize,
}

/// Public presentation value. Mutations keep identities beside their text;
/// persisted records are obtained only through the selected-session adapter.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Transcript {
    text: Vec<(String, String)>,
    pub(super) items: Vec<Item>,
    pub(super) pages: VecDeque<Page>,
    pub(super) epoch: u64,
    pub(super) local_omitted: u64,
    pub(super) protected: Option<ItemId>,
}

impl Deref for Transcript {
    type Target = [(String, String)];

    fn deref(&self) -> &Self::Target {
        &self.text
    }
}

impl PartialEq<Vec<(String, String)>> for Transcript {
    fn eq(&self, other: &Vec<(String, String)>) -> bool {
        &self.text == other
    }
}

impl From<Vec<(String, String)>> for Transcript {
    fn from(text: Vec<(String, String)>) -> Self {
        text.into_iter().collect()
    }
}

impl FromIterator<(String, String)> for Transcript {
    fn from_iter<T: IntoIterator<Item = (String, String)>>(iter: T) -> Self {
        let mut transcript = Self::default();
        for item in iter {
            transcript.push(item);
        }
        transcript
    }
}

pub(super) fn turn_key(session: &str, turn: &str, view: &str) -> String {
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

impl Transcript {
    pub fn push(&mut self, text: (String, String)) {
        self.append(
            text,
            Item {
                projection: uuid::Uuid::new_v4(),
                id: ItemId::Local(uuid::Uuid::new_v4()),
                turn_key: None,
                local: true,
            },
        );
        self.bound_locals();
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.items.clear();
        self.pages.clear();
        self.local_omitted = 0;
        self.protected = None;
        self.epoch = self.epoch.wrapping_add(1);
    }

    pub fn truncate(&mut self, length: usize) {
        self.text.truncate(length);
        self.items.truncate(length);
        self.pages.clear();
        if self
            .protected
            .as_ref()
            .is_some_and(|id| !self.items.iter().any(|item| &item.id == id))
        {
            self.protected = None;
        }
        self.epoch = self.epoch.wrapping_add(1);
    }

    pub(super) fn item_id(&self, index: usize) -> Option<&ItemId> {
        self.items.get(index).map(|item| &item.id)
    }

    pub(super) fn bind_user(&mut self, session: &str, turn: &str, view: &str) {
        if let Some(index) = self.text.iter().rposition(|text| text.0 == "user")
            && let Ok(node) = kuru_memory::public_turn_node_id(session, turn)
        {
            self.items[index].id = ItemId::Turn {
                node,
                slot: MessageSlot::User,
            };
            self.items[index].turn_key = Some(turn_key(session, turn, view));
            self.epoch = self.epoch.wrapping_add(1);
        }
    }

    pub(super) fn bind_answer(&mut self, session: &str, turn: &str, view: &str) {
        if let Some(item) = self.items.last_mut()
            && let Ok(node) = kuru_memory::public_turn_node_id(session, turn)
        {
            item.id = ItemId::Turn {
                node,
                slot: MessageSlot::Terminal(0),
            };
            item.turn_key = Some(turn_key(session, turn, view));
            self.epoch = self.epoch.wrapping_add(1);
        }
    }

    pub(super) fn older_cursor(&self) -> Option<PublicTranscriptCursor> {
        self.pages.front().and_then(|page| page.next.clone())
    }

    pub(super) fn omitted_records(&self) -> u64 {
        self.pages.front().map_or(0, |page| {
            page.total
                .saturating_sub(self.pages.iter().map(|page| page.records as u64).sum())
        })
    }

    pub(super) fn cursor_for(&self, id: &ItemId) -> Option<Option<PublicTranscriptCursor>> {
        let index = self.items.iter().position(|item| &item.id == id)?;
        let mut start = 0;
        for page in &self.pages {
            if index < start + page.items {
                return Some(page.entry.clone());
            }
            start += page.items;
        }
        None
    }

    pub(super) fn install_older(&mut self, mut page: PublicTranscriptPage) -> bool {
        let Some(old) = self.pages.front() else {
            return false;
        };
        if page.session_id != old.session
            || page.view != old.view
            || page.revision != old.revision
            || page.head_node_id != old.head
        {
            return false;
        }
        page.pending = None;
        let mut incoming = Self::from_page(page);
        incoming.pages[0].entry = old.next.clone();
        // A persisted page may now contain a formerly local completed turn.
        // Keep the canonical identity once, without counting this as omission.
        for index in (0..self.items.len()).rev() {
            if self.items[index].local
                && incoming
                    .items
                    .iter()
                    .any(|item| item.id == self.items[index].id)
            {
                self.items.remove(index);
                self.text.remove(index);
            }
        }
        incoming.text.append(&mut self.text);
        incoming.items.append(&mut self.items);
        self.text = incoming.text;
        self.items = incoming.items;
        self.pages
            .push_front(incoming.pages.pop_front().expect("projected page"));
        while self.pages.len() > 2 {
            let removed = self.pages.pop_back().expect("retained newer page");
            let start = self.pages.iter().map(|page| page.items).sum::<usize>();
            self.text.drain(start..start + removed.items);
            self.items.drain(start..start + removed.items);
        }
        self.epoch = self.epoch.wrapping_add(1);
        true
    }

    pub(super) fn next_match(
        &self,
        query: &str,
        after: Option<&(ItemId, usize)>,
    ) -> Option<(ItemId, usize)> {
        if query.is_empty() {
            return None;
        }
        let public_items = self.pages.iter().map(|page| page.items).sum::<usize>();
        let mut reached = after.is_none();
        for index in (0..public_items.min(self.len())).rev() {
            let item = &self.items[index];
            let start = if let Some((id, byte)) = after.filter(|(id, _)| *id == item.id) {
                let _ = id;
                reached = true;
                *byte
            } else if !reached {
                continue;
            } else {
                0
            };
            let body = &self.text[index].1;
            if let Some(rest) = body.get(start..)
                && let Some(byte) = rest.find(query)
            {
                return Some((item.id.clone(), start + byte));
            }
        }
        None
    }

    pub(super) fn newer_target(&self) -> Option<PublicTranscriptCursor> {
        self.pages.back().and_then(|page| page.entry.clone())
    }

    pub(super) fn install_newer(
        &mut self,
        mut page: PublicTranscriptPage,
        entry: Option<PublicTranscriptCursor>,
    ) -> bool {
        let Some(old) = self.pages.back() else {
            return false;
        };
        if page.session_id != old.session
            || page.view != old.view
            || page.revision != old.revision
            || page.head_node_id != old.head
            || page.next != old.entry
        {
            return false;
        }
        if entry.is_some() {
            page.pending = None;
        }
        let mut incoming = Self::from_page(page);
        incoming.pages[0].entry = entry;
        for index in (0..self.items.len()).rev() {
            if self.items[index].local
                && incoming
                    .items
                    .iter()
                    .any(|item| item.id == self.items[index].id)
            {
                self.items.remove(index);
                self.text.remove(index);
            }
        }
        let insert = self.pages.iter().map(|page| page.items).sum::<usize>();
        self.text.splice(insert..insert, incoming.text);
        self.items.splice(insert..insert, incoming.items);
        self.pages
            .push_back(incoming.pages.pop_front().expect("projected page"));
        while self.pages.len() > 2 {
            let removed = self.pages.pop_front().expect("retained older page");
            self.text.drain(..removed.items);
            self.items.drain(..removed.items);
        }
        self.epoch = self.epoch.wrapping_add(1);
        true
    }

    pub(super) fn install_newest(&mut self, page: PublicTranscriptPage) {
        self.install_projection(Self::from_page(page));
    }

    pub(super) fn install_projection(&mut self, incoming: Self) {
        let epoch = self.epoch;
        let omitted = self.local_omitted;
        let protected = self.protected.clone();
        let mut locals = self
            .text
            .drain(..)
            .zip(self.items.drain(..))
            .filter(|(_, item)| item.local)
            .collect::<Vec<_>>();
        *self = incoming;
        for (text, item) in locals.drain(..) {
            if !self.items.iter().any(|retained| retained.id == item.id) {
                self.append(text, item);
            }
        }
        self.protected = protected.filter(|id| self.items.iter().any(|item| &item.id == id));
        self.bound_locals();
        self.epoch = epoch.wrapping_add(1);
        self.local_omitted = self.local_omitted.saturating_add(omitted);
    }

    fn append(&mut self, text: (String, String), item: Item) {
        self.text.push(text);
        self.items.push(item);
        self.epoch = self.epoch.wrapping_add(1);
    }

    fn bound_locals(&mut self) {
        let mut count = 0usize;
        let mut bytes = 0usize;
        for (text, item) in self.text.iter().zip(&self.items).rev() {
            if item.local {
                count += 1;
                bytes = bytes
                    .saturating_add(text.0.len())
                    .saturating_add(text.1.len());
            }
        }
        while count > LOCAL_ITEMS || bytes > LOCAL_BYTES {
            let Some(index) = self
                .items
                .iter()
                .position(|item| item.local && Some(&item.id) != self.protected.as_ref())
            else {
                break;
            };
            let removed = self.text.remove(index);
            self.items.remove(index);
            bytes = bytes.saturating_sub(removed.0.len() + removed.1.len());
            count -= 1;
            self.local_omitted = self.local_omitted.saturating_add(1);
        }
    }

    pub(super) fn from_page(mut page: PublicTranscriptPage) -> Self {
        let mut transcript = Self::default();
        let records = page.records.len();
        page.records.reverse();
        for entry in page.records {
            match entry {
                PublicTranscriptEntry::Legacy { sequence, message } => {
                    transcript.append(
                        project_transcript_message(&message.role, message.text_projection()),
                        Item {
                            projection: uuid::Uuid::new_v4(),
                            id: ItemId::Legacy(sequence),
                            turn_key: None,
                            local: false,
                        },
                    );
                }
                PublicTranscriptEntry::Turn { record } => {
                    transcript.append_turn(record, &page.view);
                }
            }
        }
        let items = transcript.len();
        transcript.pages.push_back(Page {
            session: page.session_id,
            view: page.view.clone(),
            revision: page.revision,
            head: page.head_node_id,
            total: page.total_rows,
            next: page.next,
            entry: None,
            records,
            items,
        });
        if let Some(record) = page.pending {
            let first = transcript.len();
            transcript.append_turn(record, &page.view);
            for item in &mut transcript.items[first..] {
                item.local = true;
            }
        }
        transcript
    }

    fn append_turn(&mut self, record: kuru_memory::PublicTurnRecord, view: &str) {
        let key = turn_key(&record.origin_session_id, &record.turn_id, view);
        if let Some(message) = record.user_entry {
            self.append(
                project_transcript_message(&message.role, message.text_projection()),
                Item {
                    projection: uuid::Uuid::new_v4(),
                    id: ItemId::Turn {
                        node: record.node_id.clone(),
                        slot: MessageSlot::User,
                    },
                    turn_key: Some(key.clone()),
                    local: false,
                },
            );
        }
        for (slot, message) in record.terminal_entries.into_iter().enumerate() {
            let role = if message.role == "assistant" {
                record.speaker_id.as_deref().unwrap_or("unknown")
            } else {
                &message.role
            };
            self.append(
                project_transcript_message(role, message.text_projection()),
                Item {
                    projection: uuid::Uuid::new_v4(),
                    id: ItemId::Turn {
                        node: record.node_id.clone(),
                        slot: MessageSlot::Terminal(slot),
                    },
                    turn_key: Some(key.clone()),
                    local: false,
                },
            );
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct Search {
    pub(super) query: String,
    pub(super) scanned: u64,
    pub(super) status: String,
    pub(super) cursor: Option<PublicTranscriptCursor>,
    pub(super) requested: bool,
    pub(super) finished: bool,
    pub(super) saved_position: Position,
    pub(super) saved_cursor: Option<Option<PublicTranscriptCursor>>,
    pub(super) last_match: Option<(ItemId, usize)>,
}

impl Search {
    pub(super) fn new(position: Position, cursor: Option<Option<PublicTranscriptCursor>>) -> Self {
        Self {
            query: String::new(),
            scanned: 0,
            status: "Type a literal transcript search".into(),
            cursor: None,
            requested: false,
            finished: false,
            saved_position: position,
            saved_cursor: cursor,
            last_match: None,
        }
    }
    pub(super) fn changed(&mut self) {
        self.last_match = None;
        self.scanned = 0;
        self.cursor = None;
        self.finished = false;
        self.requested = !self.query.is_empty();
        self.status = "Scanning public transcript…".into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuru_core::Message;
    use kuru_memory::{
        PublicTranscriptPosition, PublicTurnKind, PublicTurnRecord, PublicTurnSettlement,
    };

    fn cursor(turn: usize) -> PublicTranscriptCursor {
        PublicTranscriptCursor {
            session_id: "session".into(),
            revision: "r".into(),
            head_node_id: Some("head".into()),
            next: PublicTranscriptPosition::Turn {
                node_id: format!("node-{turn}"),
            },
        }
    }
    fn page(turn: usize, next: Option<PublicTranscriptCursor>, user: bool) -> PublicTranscriptPage {
        PublicTranscriptPage {
            session_id: "session".into(),
            view: "main".into(),
            revision: "r".into(),
            head_node_id: Some("head".into()),
            pending: None,
            total_rows: 3,
            next,
            records: vec![PublicTranscriptEntry::Turn {
                record: PublicTurnRecord {
                    node_id: kuru_memory::public_turn_node_id("session", &turn.to_string())
                        .unwrap(),
                    origin_session_id: "session".into(),
                    turn_id: turn.to_string(),
                    kind: PublicTurnKind::Primary,
                    continuation_of_node_id: None,
                    predecessor_node_id: None,
                    settlement: PublicTurnSettlement::Completed,
                    user_entry: user.then(|| Message::text("user", format!("question-{turn}"))),
                    speaker_id: Some("speaker".into()),
                    terminal_entries: vec![Message::text("assistant", format!("answer-{turn}"))],
                    record_format: kuru_memory::PUBLIC_TURN_RECORD_FORMAT.into(),
                },
            }],
        }
    }

    #[test]
    fn page_eviction_keeps_canonical_ids_anchors_and_exact_record_counts() {
        let mut transcript = Transcript::from_page(page(2, Some(cursor(1)), true));
        let answer = transcript.item_id(1).unwrap().clone();
        assert!(transcript.install_older(page(1, Some(cursor(0)), true)));
        assert_eq!(transcript.pages.len(), 2);
        assert_eq!(transcript.omitted_records(), 1);
        let anchor = transcript.item_id(0).unwrap().clone();
        transcript.protected = Some(anchor.clone());
        assert!(transcript.install_older(page(0, None, true)));
        assert_eq!(transcript.pages.len(), 2);
        assert!(transcript.items.iter().any(|item| item.id == anchor));
        assert!(!transcript.items.iter().any(|item| item.id == answer));
        assert_eq!(transcript.newer_target(), Some(cursor(1)));
        assert!(transcript.install_newer(page(2, Some(cursor(1)), true), None));
        assert_eq!(transcript.omitted_records(), 1);
        assert!(transcript.items.iter().any(|item| item.id == answer));
    }

    #[test]
    fn literal_next_match_visits_all_offsets_before_older_records_and_preserves_local_tail() {
        let mut projected = Transcript::from_page(page(2, Some(cursor(1)), true));
        projected.text[1].1 = "needle 猫 needle".into();
        let first = projected.next_match("needle", None).unwrap();
        let second = projected
            .next_match("needle", Some(&(first.0.clone(), first.1 + 6)))
            .unwrap();
        assert_eq!(first.0, second.0);
        assert_eq!(first.1, 0);
        assert_eq!(second.1, "needle 猫 ".len());
        assert!(
            projected
                .next_match("needle", Some(&(second.0, second.1 + 6)))
                .is_none()
        );
        assert!(projected.next_match("Needle", None).is_none());
        let mut current = Transcript::from_page(page(1, None, true));
        current.push(("kuru".into(), "current-operation notice".into()));
        let notice = current.item_id(2).unwrap().clone();
        current.install_projection(projected);
        assert_eq!(current.last().unwrap().1, "current-operation notice");
        assert_eq!(current.item_id(current.len() - 1), Some(&notice));
        assert_eq!(current.pages.len(), 1);
    }

    #[test]
    fn live_and_persisted_answer_slots_match_and_local_duplicates_are_removed() {
        let mut transcript = Transcript::from_page(page(2, Some(cursor(1)), false));
        let answer = transcript.item_id(0).unwrap().clone();
        transcript.push(("speaker".into(), "answer-1".into()));
        transcript.bind_answer("session", "1", "main");
        let local = transcript.item_id(1).unwrap().clone();
        assert!(transcript.install_older(page(1, None, false)));
        assert_eq!(
            transcript
                .items
                .iter()
                .filter(|item| item.id == local)
                .count(),
            1
        );
        assert!(matches!(
            answer,
            ItemId::Turn {
                slot: MessageSlot::Terminal(0),
                ..
            }
        ));
        assert_eq!(transcript.local_omitted, 0);
        let before = transcript.clone();
        let mut stale = page(0, None, false);
        stale.revision = "changed".into();
        assert!(!transcript.install_older(stale));
        assert_eq!(transcript, before);
    }
}
