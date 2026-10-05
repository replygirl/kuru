use std::collections::VecDeque;

use unicode_segmentation::UnicodeSegmentation;

pub(super) const DRAFT_BYTE_LIMIT: usize = 131_072;
const HISTORY_PER_SESSION_LIMIT: usize = 32;
const HISTORY_TOTAL_LIMIT: usize = 128;
const HISTORY_BYTE_LIMIT: usize = 524_288;
const PASTE_CHIP_THRESHOLD: usize = 512;
const PASTE_CHIP_LIMIT: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PasteChip {
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) expanded: bool,
}

#[derive(Debug, Clone, Default)]
pub(super) struct PasteChips {
    chips: Vec<PasteChip>,
}

impl PasteChips {
    pub(super) fn snap_cursor(&self, input: &str, cursor: usize) -> usize {
        grapheme_at_or_after(input, cursor)
    }

    pub(super) fn get_at(&self, input: &str, cursor: usize) -> Option<&PasteChip> {
        self.chip_at_cursor(input, cursor)
    }

    fn chip_at_cursor(&self, input: &str, cursor: usize) -> Option<&PasteChip> {
        let cursor = grapheme_at_or_after(input, cursor);
        let previous = previous_grapheme(input, cursor);
        let next = next_grapheme(input, cursor);
        self.chips.iter().find(|chip| {
            (chip.start <= cursor && cursor <= chip.end)
                || (chip.start < cursor && chip.end > previous)
                || (chip.start < next && chip.end > cursor)
        })
    }

    pub(super) fn collapsed_starting_at(&self, cursor: usize) -> Option<&PasteChip> {
        self.chips
            .iter()
            .find(|chip| !chip.expanded && chip.start == cursor)
    }

    pub(super) fn collapsed_ending_at(&self, cursor: usize) -> Option<&PasteChip> {
        self.chips
            .iter()
            .find(|chip| !chip.expanded && chip.end == cursor)
    }

    pub(super) fn toggle_at(&mut self, input: &str, cursor: usize) -> bool {
        let cursor = grapheme_at_or_after(input, cursor);
        let previous = previous_grapheme(input, cursor);
        let next = next_grapheme(input, cursor);
        let Some(index) = self.chips.iter().position(|chip| {
            (chip.start <= cursor && cursor <= chip.end)
                || (chip.start < cursor && chip.end > previous)
                || (chip.start < next && chip.end > cursor)
        }) else {
            return false;
        };
        self.chips[index].expanded = !self.chips[index].expanded;
        true
    }

    pub(super) fn remove_at(&mut self, input: &str, cursor: usize) -> Option<PasteChip> {
        let chip = self.chip_at_cursor(input, cursor)?;
        let index = self.chips.iter().position(|candidate| candidate == chip)?;
        Some(self.chips.remove(index))
    }

    pub(super) fn expand_before_edit(&mut self, start: usize, end: usize) -> bool {
        let mut expanded = false;
        for chip in &mut self.chips {
            let intersects = (start < chip.end && end > chip.start)
                || (start == end && chip.start < start && start < chip.end);
            if intersects && !chip.expanded {
                chip.expanded = true;
                expanded = true;
            }
        }
        expanded
    }

    pub(super) fn apply_edit(&mut self, start: usize, end: usize, replacement_len: usize) {
        let old_len = end - start;
        let mut next = Vec::with_capacity(self.chips.len());
        for mut chip in self.chips.drain(..) {
            let intersects = (start < chip.end && end > chip.start)
                || (start == end && chip.start < start && start < chip.end);
            if intersects {
                continue;
            }
            if chip.start >= end {
                if replacement_len >= old_len {
                    let delta = replacement_len - old_len;
                    chip.start += delta;
                    chip.end += delta;
                } else {
                    let delta = old_len - replacement_len;
                    chip.start -= delta;
                    chip.end -= delta;
                }
            }
            next.push(chip);
        }
        self.chips = next;
    }

    pub(super) fn add_paste(&mut self, start: usize, text: &str) -> bool {
        if text.len() < PASTE_CHIP_THRESHOLD
            && text.bytes().filter(|byte| *byte == b'\n').count() < 3
        {
            return false;
        }
        if self.chips.len() == PASTE_CHIP_LIMIT {
            return false;
        }
        self.chips.push(PasteChip {
            start,
            end: start + text.len(),
            expanded: false,
        });
        self.chips.sort_by_key(|chip| chip.start);
        true
    }

    pub(super) fn clear(&mut self) {
        self.chips.clear();
    }

    pub(super) fn project(&self, input: &str, cursor: usize) -> (String, usize) {
        let mut visible = String::with_capacity(input.len().min(DRAFT_BYTE_LIMIT));
        let mut projected_cursor = 0;
        let mut consumed = 0;
        for chip in &self.chips {
            if chip.start < consumed
                || chip.end > input.len()
                || !input.is_char_boundary(chip.start)
                || !input.is_char_boundary(chip.end)
            {
                continue;
            }
            visible.push_str(&input[consumed..chip.start]);
            if cursor >= consumed && cursor <= chip.start {
                projected_cursor = visible.len() - (chip.start - cursor);
            }
            if chip.expanded {
                visible.push_str(&input[chip.start..chip.end]);
                if cursor > chip.start && cursor <= chip.end {
                    projected_cursor = visible.len() - (chip.end - cursor);
                }
            } else {
                let original = &input[chip.start..chip.end];
                let lines = original.bytes().filter(|byte| *byte == b'\n').count() + 1;
                visible.push_str(&format!(
                    "[paste · {} bytes · {lines} lines]",
                    original.len()
                ));
                if cursor > chip.start && cursor <= chip.end {
                    projected_cursor = visible.len();
                }
            }
            consumed = chip.end;
        }
        visible.push_str(&input[consumed..]);
        if cursor >= consumed {
            projected_cursor = visible.len() - (input.len() - cursor.min(input.len()));
        }
        (visible, projected_cursor)
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct SubmittedHistory {
    entries: VecDeque<(String, String)>,
    bytes: usize,
}

impl SubmittedHistory {
    pub(super) fn record(&mut self, session: &str, prompt: &str) -> bool {
        if prompt.is_empty() || prompt.len() > DRAFT_BYTE_LIMIT {
            return false;
        }
        self.bytes += prompt.len();
        self.entries
            .push_back((session.to_owned(), prompt.to_owned()));
        let mut evicted = false;
        while self
            .entries
            .iter()
            .filter(|(entry_session, _)| entry_session == session)
            .count()
            > HISTORY_PER_SESSION_LIMIT
        {
            let oldest_in_session = self
                .entries
                .iter()
                .position(|(entry_session, _)| entry_session == session)
                .unwrap();
            let (_, old) = self.entries.remove(oldest_in_session).unwrap();
            self.bytes -= old.len();
            evicted = true;
        }
        while self.entries.len() > HISTORY_TOTAL_LIMIT || self.bytes > HISTORY_BYTE_LIMIT {
            if let Some((_, old)) = self.entries.pop_front() {
                self.bytes -= old.len();
                evicted = true;
            }
        }
        evicted
    }

    pub(super) fn matches(&self, session: &str, query: &str) -> Vec<String> {
        self.entries
            .iter()
            .rev()
            .filter(|(entry_session, prompt)| entry_session == session && prompt.contains(query))
            .map(|(_, prompt)| prompt.clone())
            .collect()
    }
}

pub(super) fn grapheme_at_or_after(text: &str, cursor: usize) -> usize {
    let cursor = cursor.min(text.len());
    for (start, grapheme) in text.grapheme_indices(true) {
        if start >= cursor {
            return start;
        }
        if start + grapheme.len() > cursor {
            return start + grapheme.len();
        }
    }
    text.len()
}

pub(super) fn previous_grapheme(text: &str, cursor: usize) -> usize {
    let cursor = grapheme_at_or_after(text, cursor);
    text[..cursor]
        .grapheme_indices(true)
        .next_back()
        .map_or(0, |(start, _)| start)
}

pub(super) fn next_grapheme(text: &str, cursor: usize) -> usize {
    let cursor = grapheme_at_or_after(text, cursor);
    text[cursor..]
        .graphemes(true)
        .next()
        .map_or(text.len(), |grapheme| cursor + grapheme.len())
}

pub(super) fn line_start(text: &str, cursor: usize) -> usize {
    text[..grapheme_at_or_after(text, cursor)]
        .rfind('\n')
        .map_or(0, |index| index + 1)
}

pub(super) fn line_end(text: &str, cursor: usize) -> usize {
    let cursor = grapheme_at_or_after(text, cursor);
    let end = text[cursor..]
        .find('\n')
        .map_or(text.len(), |offset| cursor + offset);
    if end > 0 && text.as_bytes()[end - 1] == b'\r' {
        end - 1
    } else {
        end
    }
}

pub(super) fn vertical_target(text: &str, cursor: usize, up: bool) -> Option<usize> {
    let cursor = grapheme_at_or_after(text, cursor);
    let current_start = line_start(text, cursor);
    let current_end = line_end(text, cursor);
    let column = text[current_start..cursor.min(current_end)]
        .graphemes(true)
        .count();
    let (target_start, target_end) = if up {
        if current_start == 0 {
            return None;
        }
        let previous_end = if current_start >= 2 && text.as_bytes()[current_start - 2] == b'\r' {
            current_start - 2
        } else {
            current_start - 1
        };
        (line_start(text, previous_end), previous_end)
    } else {
        if current_end == text.len() {
            return None;
        }
        let next_start = if text[current_end..].starts_with("\r\n") {
            current_end + 2
        } else {
            current_end + 1
        };
        (next_start, line_end(text, next_start))
    };
    let mut target = target_start;
    for grapheme in text[target_start..target_end].graphemes(true).take(column) {
        target += grapheme.len();
    }
    Some(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motions_keep_combining_and_joined_emoji_whole() {
        let input = "e\u{301} 👩‍💻\n猫";
        let end = input.len();
        assert_eq!(previous_grapheme(input, end), input.find('猫').unwrap());
        let line = line_start(input, end);
        assert_eq!(&input[line..], "猫");
        assert_eq!(line_end(input, 0), input.find('\n').unwrap());
        let joined_end = input.find('\n').unwrap();
        let joined_start = previous_grapheme(input, joined_end);
        assert_eq!(&input[joined_start..joined_end], "👩‍💻");
        assert_eq!(next_grapheme(input, 0), "e\u{301}".len());
    }

    #[test]
    fn submitted_history_is_session_scoped_and_eviction_is_explicit() {
        let mut history = SubmittedHistory::default();
        assert!(!history.record("a", "alpha"));
        assert!(!history.record("b", "bravo"));
        assert_eq!(history.matches("a", ""), vec!["alpha"]);
        assert_eq!(history.matches("b", ""), vec!["bravo"]);
        for i in 0..HISTORY_PER_SESSION_LIMIT {
            history.record("a", &format!("alpha-{i}"));
        }
        assert_eq!(history.matches("a", "").len(), HISTORY_PER_SESSION_LIMIT);
        assert!(!history.matches("a", "").contains(&"alpha".to_owned()));
        assert_eq!(history.matches("b", ""), vec!["bravo"]);

        let mut bounded = SubmittedHistory::default();
        for index in 0..HISTORY_TOTAL_LIMIT + 1 {
            bounded.record(&format!("session-{index}"), &format!("prompt-{index}"));
        }
        assert_eq!(bounded.entries.len(), HISTORY_TOTAL_LIMIT);
        assert!(bounded.matches("session-0", "").is_empty());

        let mut byte_bounded = SubmittedHistory::default();
        let maximum = "x".repeat(DRAFT_BYTE_LIMIT);
        for index in 0..4 {
            byte_bounded.record(&format!("session-{index}"), &maximum);
        }
        assert_eq!(byte_bounded.bytes, HISTORY_BYTE_LIMIT);
        byte_bounded.record("session-4", "new");
        assert!(byte_bounded.matches("session-0", "").is_empty());
        assert!(byte_bounded.bytes <= HISTORY_BYTE_LIMIT);
    }

    #[test]
    fn vertical_motion_respects_grapheme_columns_and_crlf_breaks() {
        let input = "a👩‍💻b\r\n猫x\nlast";
        let first_line_end = input.find('\r').unwrap();
        let second_line_start = input.find('猫').unwrap();
        assert_eq!(
            vertical_target(input, first_line_end, false),
            Some(second_line_start + "猫x".len())
        );
        assert_eq!(vertical_target(input, second_line_start, true), Some(0));
        assert_eq!(vertical_target(input, 0, true), None);
        assert_eq!(vertical_target(input, input.len(), false), None);
    }

    #[test]
    fn paste_projection_never_replaces_canonical_text() {
        let paste = "猫\n".repeat(300);
        let input = format!("before{paste}after");
        let start = "before".len();
        let end = start + paste.len();
        let mut chips = PasteChips::default();
        assert!(chips.add_paste(start, &paste));
        let (compact, at_end) = chips.project(&input, end);
        assert!(compact.starts_with("before[paste · "));
        assert!(compact.ends_with("after"));
        assert!(!compact.contains("猫"));
        assert_eq!(&compact[at_end..], "after");
        assert!(chips.toggle_at(&input, end));
        let (expanded, _) = chips.project(&input, end);
        assert_eq!(expanded, input);
        let removed = chips.remove_at(&input, end).unwrap();
        chips.apply_edit(removed.start, removed.end, 0);
        let mut remainder = input.clone();
        remainder.replace_range(removed.start..removed.end, "");
        assert_eq!(remainder, "beforeafter");
        assert!(chips.get_at(&input, start).is_none());
    }

    #[test]
    fn pasted_combining_boundary_stays_exact_and_chip_is_reachable() {
        let pasted = "\u{301}".repeat(300);
        let input = format!("e{pasted}tail");
        let start = 1;
        let end = start + pasted.len();
        let mut chips = PasteChips::default();
        assert!(chips.add_paste(start, &pasted));
        let after_joined_grapheme = next_grapheme(&input, 0);
        assert_eq!(after_joined_grapheme, end);
        assert!(chips.get_at(&input, after_joined_grapheme).is_some());
        assert!(chips.toggle_at(&input, after_joined_grapheme));
        let removed = chips.remove_at(&input, after_joined_grapheme).unwrap();
        assert_eq!((removed.start, removed.end), (start, end));
        let mut remainder = input;
        remainder.replace_range(removed.start..removed.end, "");
        assert_eq!(remainder, "etail");
    }
}
