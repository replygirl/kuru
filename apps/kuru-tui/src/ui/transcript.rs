//! Owned public display identities and bounded retained transcript pages.

use std::{
    collections::{BTreeSet, VecDeque},
    ops::Deref,
    sync::atomic::{AtomicU64, Ordering},
};

use kuru_memory::{PublicTranscriptCursor, PublicTranscriptEntry, PublicTranscriptPage};
use sha2::{Digest, Sha256};

use super::project_transcript_message;

pub(super) const PAGE_RECORDS: usize = 32;
const LOCAL_ITEMS: usize = 128;
const LOCAL_BYTES: usize = 32 * 1024 * 1024;

/// Creation order of local items across every projection; positions no
/// longer imply age once locals are re-placed by anchor.
static LOCAL_SEQUENCE: AtomicU64 = AtomicU64::new(1);

fn next_local_sequence() -> u64 {
    LOCAL_SEQUENCE.fetch_add(1, Ordering::Relaxed)
}

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
    /// For a local item: the last persisted-identity item displayed before it
    /// (`None` when it preceded every record). Page installs re-place the item
    /// after this anchor instead of moving it to the tail.
    anchor: Option<ItemId>,
    /// Local creation order (0 for public items); the bound evicts by age.
    sequence: u64,
}

/// Locals removed before public-page arithmetic, with the previous order of
/// every item so a local whose anchor left the window keeps its relative place.
struct Detached {
    order: Vec<ItemId>,
    locals: Vec<((String, String), Item)>,
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
        let anchor = self.persisted_tail();
        self.append(
            text,
            Item {
                projection: uuid::Uuid::new_v4(),
                id: ItemId::Local(uuid::Uuid::new_v4()),
                turn_key: None,
                local: true,
                anchor,
                sequence: next_local_sequence(),
            },
        );
        self.bound_locals();
    }

    /// The last item with a persisted identity, including in-flight user and
    /// answer items already bound to their turn.
    fn persisted_tail(&self) -> Option<ItemId> {
        self.items
            .iter()
            .rev()
            .find(|item| !matches!(item.id, ItemId::Local(_)))
            .map(|item| item.id.clone())
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
        // Pages count public items only; locals have no page cursor.
        let index = self
            .items
            .iter()
            .filter(|item| !item.local)
            .position(|item| &item.id == id)?;
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
        // Public-page arithmetic below sees only public items; a persisted page
        // may also now contain a formerly local completed turn, which
        // `attach` keeps once, without counting it as omission.
        let detached = self.detach();
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
        self.attach(detached);
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
        let mut reached = after.is_none();
        for index in (0..self.len()).rev() {
            let item = &self.items[index];
            if item.local {
                continue;
            }
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
        let detached = self.detach();
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
        self.attach(detached);
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
        let detached = self.detach();
        *self = incoming;
        self.attach(detached);
        self.protected = protected.filter(|id| self.items.iter().any(|item| &item.id == id));
        self.bound_locals();
        self.epoch = epoch.wrapping_add(1);
        self.local_omitted = self.local_omitted.saturating_add(omitted);
    }

    /// Remove every local item, leaving public items contiguous by page.
    fn detach(&mut self) -> Detached {
        let order = self.items.iter().map(|item| item.id.clone()).collect();
        let mut locals = Vec::new();
        let mut index = 0;
        while index < self.items.len() {
            if self.items[index].local {
                locals.push((self.text.remove(index), self.items.remove(index)));
            } else {
                index += 1;
            }
        }
        Detached { order, locals }
    }

    /// Re-place detached locals in their previous order: after their anchor
    /// (and after locals already placed there), before every public record
    /// when they preceded all of them, before the next surviving later item
    /// when the anchor left this window, and otherwise at the end. A local
    /// whose identity the incoming items already hold (a bound turn now
    /// persisted) is replaced by that record.
    fn attach(&mut self, detached: Detached) {
        let Detached { order, locals } = detached;
        let mut placed = BTreeSet::new();
        let mut front = 0usize;
        for (text, item) in locals {
            if self.items.iter().any(|retained| retained.id == item.id) {
                continue;
            }
            let position = |id: &ItemId| self.items.iter().position(|retained| &retained.id == id);
            let index = match &item.anchor {
                None => front,
                Some(anchor) => {
                    if let Some(at) = position(anchor) {
                        let mut index = at + 1;
                        while self
                            .items
                            .get(index)
                            .is_some_and(|next| placed.contains(&next.id))
                        {
                            index += 1;
                        }
                        index
                    } else {
                        // Linear in the saved order per orphaned local; both
                        // are bounded by LOCAL_ITEMS plus two public pages.
                        order
                            .iter()
                            .skip_while(|id| **id != item.id)
                            .skip(1)
                            .find_map(position)
                            .unwrap_or(self.items.len())
                    }
                }
            };
            if index <= front {
                front += 1;
            }
            placed.insert(item.id.clone());
            self.text.insert(index, text);
            self.items.insert(index, item);
        }
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
            // Evict the oldest local by creation order, not by position.
            let Some(index) = self
                .items
                .iter()
                .enumerate()
                .filter(|(_, item)| item.local && Some(&item.id) != self.protected.as_ref())
                .min_by_key(|(_, item)| item.sequence)
                .map(|(index, _)| index)
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
                            anchor: None,
                            sequence: 0,
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
            let anchor = transcript.persisted_tail();
            transcript.append_turn(record, &page.view);
            for item in &mut transcript.items[first..] {
                item.local = true;
                item.anchor = anchor.clone();
                item.sequence = next_local_sequence();
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
                    anchor: None,
                    sequence: 0,
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
                    anchor: None,
                    sequence: 0,
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

    /// A newest page holding `turns` (oldest first), as the reader returns it.
    fn newest(turns: &[usize]) -> PublicTranscriptPage {
        let mut records = turns
            .iter()
            .flat_map(|turn| page(*turn, None, true).records)
            .collect::<Vec<_>>();
        records.reverse();
        PublicTranscriptPage {
            total_rows: records.len() as u64,
            records,
            ..page(0, None, true)
        }
    }

    fn texts(transcript: &Transcript) -> Vec<&str> {
        transcript.iter().map(|(_, text)| text.as_str()).collect()
    }

    #[test]
    fn help_output_keeps_its_place_when_the_next_turn_is_persisted() {
        let mut transcript = Transcript::from_page(newest(&[1]));
        transcript.push(("help".into(), "help output".into()));
        transcript.push(("user".into(), "question-2".into()));
        transcript.bind_user("session", "2", "main");
        transcript.push(("speaker".into(), "answer-2".into()));
        transcript.bind_answer("session", "2", "main");
        transcript.install_newest(newest(&[1, 2]));
        assert_eq!(
            texts(&transcript),
            [
                "question-1",
                "answer-1",
                "help output",
                "question-2",
                "answer-2"
            ]
        );
        transcript.install_newest(newest(&[1, 2, 3]));
        assert_eq!(
            texts(&transcript),
            [
                "question-1",
                "answer-1",
                "help output",
                "question-2",
                "answer-2",
                "question-3",
                "answer-3"
            ]
        );
    }

    #[test]
    fn error_lines_and_notices_after_an_answer_stay_after_that_answer() {
        let mut transcript = Transcript::from_page(newest(&[1]));
        transcript.push(("user".into(), "question-2".into()));
        transcript.bind_user("session", "2", "main");
        transcript.push(("speaker".into(), "answer-2".into()));
        transcript.bind_answer("session", "2", "main");
        transcript.push(("kuru".into(), "omission notice".into()));
        transcript.push(("user".into(), "question-3".into()));
        transcript.bind_user("session", "3", "main");
        transcript.push(("error".into(), "turn failed".into()));
        // Turn 3 failed before persistence: its local user item and error stay.
        transcript.install_newest(newest(&[1, 2]));
        assert_eq!(
            texts(&transcript),
            [
                "question-1",
                "answer-1",
                "question-2",
                "answer-2",
                "omission notice",
                "question-3",
                "turn failed"
            ]
        );
        assert_eq!(transcript.local_omitted, 0);
    }

    #[test]
    fn locals_without_records_before_them_stay_first_and_vanished_anchors_keep_relative_place() {
        let mut transcript = Transcript::default();
        transcript.push(("help".into(), "first help".into()));
        transcript.push(("status".into(), "first status".into()));
        transcript.install_newest(newest(&[1, 2]));
        assert_eq!(
            texts(&transcript),
            [
                "first help",
                "first status",
                "question-1",
                "answer-1",
                "question-2",
                "answer-2"
            ]
        );

        let mut transcript = Transcript::from_page(newest(&[1]));
        transcript.push(("help".into(), "old help".into()));
        transcript.install_newest(newest(&[1, 2, 3]));
        // The anchor scrolls out of the newest window: the help output stays
        // above the later surviving records instead of rejoining the tail.
        transcript.install_newest(newest(&[2, 3, 4]));
        assert_eq!(
            texts(&transcript),
            [
                "old help",
                "question-2",
                "answer-2",
                "question-3",
                "answer-3",
                "question-4",
                "answer-4"
            ]
        );
        // The retained anchor restores the exact place when it returns.
        transcript.install_newest(newest(&[1, 2]));
        assert_eq!(
            texts(&transcript),
            [
                "question-1",
                "answer-1",
                "old help",
                "question-2",
                "answer-2"
            ]
        );
    }

    #[test]
    fn interleaved_locals_do_not_change_public_page_eviction_cursors_or_search() {
        let mut transcript = Transcript::from_page(page(2, Some(cursor(1)), true));
        let newest_answer = transcript.item_id(1).unwrap().clone();
        transcript.push(("help".into(), "needle in help".into()));
        let help = transcript.item_id(2).unwrap().clone();
        assert!(transcript.install_older(page(1, Some(cursor(0)), true)));
        assert_eq!(
            texts(&transcript),
            [
                "question-1",
                "answer-1",
                "question-2",
                "answer-2",
                "needle in help"
            ]
        );
        assert_eq!(transcript.cursor_for(&help), None);
        let older_question = transcript.item_id(0).unwrap().clone();
        assert_eq!(
            transcript.cursor_for(&older_question),
            Some(Some(cursor(1)))
        );
        assert_eq!(transcript.cursor_for(&newest_answer), Some(None));
        assert!(transcript.next_match("needle", None).is_none());
        assert_eq!(
            transcript.next_match("answer-2", None).map(|found| found.0),
            Some(newest_answer.clone())
        );
        // Evicting the newest page keeps the local; its anchor left the
        // window and nothing later survives, so it waits at the end.
        assert!(transcript.install_older(page(0, None, true)));
        assert_eq!(
            texts(&transcript),
            [
                "question-0",
                "answer-0",
                "question-1",
                "answer-1",
                "needle in help"
            ]
        );
        assert_eq!(transcript.pages.len(), 2);
        assert_eq!(transcript.omitted_records(), 1);
        // The newer page returns after its anchor, and the oldest page leaves.
        assert!(transcript.install_newer(page(2, Some(cursor(1)), true), None));
        assert_eq!(
            texts(&transcript),
            [
                "question-1",
                "answer-1",
                "question-2",
                "answer-2",
                "needle in help"
            ]
        );
        assert_eq!(transcript.cursor_for(&newest_answer), Some(None));
    }

    #[test]
    fn local_bounds_and_omission_counts_survive_reanchoring() {
        let mut transcript = Transcript::from_page(newest(&[1]));
        for index in 0..LOCAL_ITEMS + 2 {
            transcript.push(("kuru".into(), format!("local-{index}")));
        }
        assert_eq!(transcript.local_omitted, 2);
        assert_eq!(transcript.len(), 2 + LOCAL_ITEMS);
        transcript.install_newest(newest(&[1, 2]));
        assert_eq!(transcript.local_omitted, 2);
        assert_eq!(transcript.len(), 4 + LOCAL_ITEMS);
        assert_eq!(transcript[2].1, "local-2");
        assert_eq!(
            transcript[2 + LOCAL_ITEMS - 1].1,
            format!("local-{}", LOCAL_ITEMS + 1)
        );
        assert_eq!(transcript[2 + LOCAL_ITEMS].1, "question-2");
    }

    #[test]
    fn local_bound_evicts_the_oldest_local_not_the_first_positioned() {
        let mut unanchored = Transcript::default();
        unanchored.push(("help".into(), "unanchored".into()));
        unanchored.install_newest(newest(&[1]));
        for index in 0..LOCAL_ITEMS {
            unanchored.push(("kuru".into(), format!("later-{index}")));
        }
        // The unanchored local is first by position and also oldest.
        assert_eq!(unanchored.local_omitted, 1);
        assert!(!unanchored.iter().any(|(_, text)| text == "unanchored"));

        // A newer local re-placed in front of an older one, as an unanchored
        // or orphaned local can be: the bound evicts the older, not the first.
        let mut mixed = Transcript::from_page(newest(&[1]));
        mixed.push(("kuru".into(), "older tail".into()));
        let older = mixed.item_id(2).unwrap().clone();
        let mut front = Transcript::default();
        front.push(("help".into(), "newer front".into()));
        let (text, item) = (front.text.remove(0), front.items.remove(0));
        mixed.text.insert(0, text);
        mixed.items.insert(0, item);
        for index in 0..LOCAL_ITEMS - 1 {
            mixed.push(("kuru".into(), format!("fill-{index}")));
        }
        assert_eq!(mixed.local_omitted, 1);
        assert_eq!(mixed[0].1, "newer front");
        assert!(!mixed.items.iter().any(|item| item.id == older));
    }

    #[test]
    fn pending_turn_items_follow_the_last_record_and_yield_to_the_persisted_turn() {
        let mut pending = newest(&[1]);
        pending.pending = page(2, None, true).records.pop().map(|entry| match entry {
            PublicTranscriptEntry::Turn { record } => record,
            PublicTranscriptEntry::Legacy { .. } => unreachable!("turn fixture"),
        });
        let mut transcript = Transcript::from_page(pending);
        transcript.push(("help".into(), "help output".into()));
        assert_eq!(
            texts(&transcript),
            [
                "question-1",
                "answer-1",
                "question-2",
                "answer-2",
                "help output"
            ]
        );
        transcript.install_newest(newest(&[1, 2]));
        assert_eq!(
            texts(&transcript),
            [
                "question-1",
                "answer-1",
                "question-2",
                "answer-2",
                "help output"
            ]
        );
        assert!(transcript.items.iter().filter(|item| item.local).count() == 1);
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
        // Anchored to turn 1, which the projected search page does not hold,
        // and nothing later survives: the notice keeps the tail.
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
