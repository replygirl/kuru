# interactive-composer Specification

## Purpose
The interactive composer edits and submits canonical multiline prompts in the terminal with grapheme-safe navigation, temporary per-session recall, literal-preserving paste controls, and bounded scrolling.

## Requirements

### Requirement: Grapheme-safe multiline editing

The interactive composer SHALL move, insert, and delete at extended grapheme-cluster boundaries, including combining marks and joined emoji, while retaining the exact UTF-8 draft. It SHALL support vertical column-preserving movement, line-local start/end, whole-draft start/end, and deliberate multiline insertion. Cursor placement and editing MUST use the same displayed projection and MUST NOT submit a visual label in place of canonical draft text.

#### Scenario: Edit Unicode across line boundaries
- **WHEN** a user moves through and edits a draft containing a combining sequence, a joined emoji, CRLF, and multiple lines
- **THEN** each cursor motion and deletion stays on an intended grapheme boundary and submission contains exactly the resulting literal draft.

### Requirement: Bounded selected-session prompt recall

The TUI SHALL provide navigation and Ctrl-R search over prompts dispatched from the currently selected session during the current application lifetime. History SHALL be in memory only, bounded per session and across sessions by entry count and bytes, and MUST NOT read or mutate durable transcripts, private peer histories, or memory storage. The session captured when a prompt is dispatched SHALL own its history entry even if the selected view changes before the response settles. Entering, browsing, or cancelling recall/search SHALL preserve the unsent draft and cursor until the user explicitly accepts a recalled prompt; eviction SHALL be visible and MUST NOT alter the active draft.

#### Scenario: Cancel reverse search with an unsent draft
- **WHEN** a user starts Ctrl-R with an unsent draft, browses matches, and cancels
- **THEN** the exact draft, cursor, and paste presentation return with no additional prompt dispatch or durable write.

#### Scenario: Recall after changing sessions
- **WHEN** a user dispatches prompts in two sessions and then recalls from each selected session
- **THEN** each recall list contains only prompts dispatched while that session was selected, regardless of later asynchronous response settlement.

### Requirement: Literal-preserving paste chips

The composer SHALL retain accepted paste bytes and line endings in its canonical draft and MAY render a large paste span as a compact chip with byte and line cues. Chip ranges SHALL retain the exact pasted UTF-8 byte span even when its boundary combines with neighboring text into a grapheme. Cursor motion SHALL remain on complete grapheme boundaries; chip controls SHALL remain reachable from an adjacent cursor position without widening the stored or removed span. Users SHALL be able to expand or compact a chip and explicitly remove its exact pasted span; surrounding text MUST remain unchanged. An edit intersecting a collapsed chip SHALL reveal its literal text before changing it. A paste that would make the draft exceed 128 KiB SHALL be rejected atomically with a visible notice. Submission MUST pass canonical draft text, including Unicode and line endings, rather than the chip projection.

#### Scenario: Expand and remove a large paste
- **WHEN** a draft contains text around a collapsed multiline paste and the user expands, edits, or removes that paste
- **THEN** expansion changes only the view, an edit reveals the literal span first, explicit removal deletes only that span, and the remaining draft and cursor stay coherent.

#### Scenario: Reject an oversized paste
- **WHEN** a pasted block would exceed the 128 KiB draft limit
- **THEN** no part of the block is inserted and the visible draft remains unchanged with a rejection notice.

#### Scenario: Submit a compact paste
- **WHEN** a user submits a draft containing a collapsed paste chip
- **THEN** the runtime receives the exact canonical draft bytes in their original order, including grapheme sequences and line endings.

#### Scenario: Chip boundaries join neighboring graphemes
- **WHEN** a pasted span begins with a combining mark or joined-sequence continuation that combines with adjacent draft text, and the user expands or removes that chip from a reachable cursor position
- **THEN** cursor motion remains on complete graphemes, expansion changes only the view, and removal deletes exactly the pasted bytes while preserving neighboring bytes.

### Requirement: Clamped composer viewport and modal draft ownership

The composer SHALL keep the cursor visible within the input viewport after editing, paste, and terminal resize, including practical narrow and wide terminal sizes. Transcript and composer scroll positions SHALL be clamped to current content and viewport bounds after content or size changes. Model and session pickers, instruction review, permission approval, interruption, and session rename SHALL preserve the unsent draft, cursor, and paste spans. Approval controls SHALL retain input priority: choices 1–4 and supported Alt aliases act only while the approval prompt owns input; unavailable choices MUST NOT grant authority or edit the draft, and dismissed modal keys SHALL return to ordinary composer behavior.

#### Scenario: Resize a multiline draft with an overlay
- **WHEN** a user edits a long multiline draft, resizes between narrow and wide PTY dimensions, and opens and dismisses a picker or approval prompt
- **THEN** the cursor remains visible, scroll stays within current bounds, and the exact draft, paste spans, and approval decision remain intact.

#### Scenario: Dismiss an unavailable approval choice
- **WHEN** an approval prompt disables choices 2 and 3, the user presses those keys, denies with 4, then types a digit
- **THEN** the disabled choices neither grant approval nor change the draft, and the later digit is inserted after the prompt is dismissed.
