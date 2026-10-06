# interactive-transcript-navigation Specification

## Purpose
Provide bounded selected-session public scrollback, stable reading anchors, literal incremental search and safely owned terminal navigation state without exposing private actor history.

## Requirements

### Requirement: Bounded selected-session public scrollback

The interactive transcript SHALL fetch bounded public pages through the selected-session adapter, retain exact source view/revision/cursor provenance and stable public message identities, and retain only a bounded adjacent-page window plus bounded current-operation/local display state. Rendering MUST produce viewport rows without representing the complete session by a u16 scroll offset or materializing all wrapped history. Omission notices SHALL distinguish stored public records from projected messages and disclose exact omitted public-record counts where the captured source provides them. Public readers MUST preserve cursor reachability, fork-prefix isolation and revision validation while allowing usable navigation and other-session writes. Sequential issued continuations at an unchanged selected cut SHALL reuse a completed chain proof so metadata work is one initial chain walk plus bounded page work, rather than a full chain walk per page. Any retained proof MUST be view/attachment-local, at most 2 KiB with four positions from two returned pages, retain no bodies or native read resource, and publish only after successful transaction and final response validation. Optimization bounds MUST NOT reject otherwise valid requests.

#### Scenario: Reading beyond the initial page

- **WHEN** a reader navigates a long session beyond the initially fetched public page
- **THEN** bounded older pages become visible with unchanged stored records and exact captured provenance, while evicted pages do not accumulate in the view

#### Scenario: Immutable reader permits another session to write

- **WHEN** a page traverses a long selected-session chain while another session drives the same project
- **THEN** it reads one proven selected-view committed revision outside the mutable writer lock, verifies full chain structure and exact totals using metadata or a completed exact-cut proof, and decodes only the returned bounded page bodies with unchanged public-message privacy checks

#### Scenario: Private or forged continuation

- **WHEN** a continuation names an unreachable turn, another session or a changed revision
- **THEN** it is rejected and the UI shows a stale or unavailable source state rather than exposing unrelated public or private history

#### Scenario: Sequential bounded proof reuse

- **WHEN** a reader follows issued continuations at the same checked selected revision
- **THEN** one complete metadata proof establishes the chain and totals, subsequent pages inspect only bounded requested page metadata and bodies, and unremembered coordinates fall back to full reachability proof

#### Scenario: Cancelled partial read

- **WHEN** a read is cancelled or fails before successful transaction and final response validation
- **THEN** it cannot publish a partial new proof or remove the existing source/privacy checks

### Requirement: Stable reading anchor and distinct follow-tail

Reading mode SHALL retain a stable public/local item identity and wrapped row within that item. Streaming replacement, new completion notices, variable-height tool cards and resize MUST preserve the selected item and clamp within it when its own layout shrinks. Follow-tail SHALL be a distinct state; new output MUST NOT move a reader who has left it. Transient tool cards SHALL attach through the exact admitted turn/session/view binding rather than projected display names or mutable vector indices.

#### Scenario: Variable height while reading

- **WHEN** a card above a reader expands or a streamed item changes during resize
- **THEN** the reader remains on the same identified item and valid wrapped row, while follow-tail continues to show the current tail

### Requirement: Literal incremental public transcript search

Transcript search SHALL perform case-sensitive literal matching over the selected session's public message text one bounded page at a time. It SHALL show scanned progress and distinct found, completed, cancelled and stale/unavailable states. Query, session, view and operation epochs MUST prevent stale results from publishing. Search MUST NOT hold the Harness mutex for an entire scan, materialize the complete session, query private actor history or invent unavailable historical transient-card bodies. Cancel SHALL preserve the literal composer draft and prior reading anchor.

#### Scenario: Match outside the viewport

- **WHEN** a literal match occurs in an older fetched public page outside the viewport
- **THEN** the matching public item can be shown at a stable anchor with honest scanned progress and bounded retained search state

#### Scenario: Cancel or source change

- **WHEN** search is cancelled, its query changes, the session changes or the captured revision becomes stale
- **THEN** obsolete work cannot publish a match or a completed no-match claim into the new view

### Requirement: Existing input priority with default mouse navigation

Mouse capture SHALL be enabled by default and restored by the existing terminal ownership guard. Transcript mouse/search/navigation controls MUST preserve approval and instruction choices, picker/rename overlays, recall navigation/cancellation, literal composer editing and U2 card controls. Composer Home/End SHALL retain their existing line-local behavior.

#### Scenario: Modal interaction

- **WHEN** a permission choice, picker or recall overlay owns input
- **THEN** transcript search or scrolling cannot consume its established keys or mutate the literal underlying draft

### Requirement: Safe owned terminal integration

Session titles SHALL contain only a fixed application prefix and sanitized bounded session metadata. A title SHALL be changed only through a mechanism that supports restoring the actual previous title; unsupported mechanisms SHALL be skipped rather than reset to a guessed default. Successful nonreused turn completion while unfocused MAY emit one fixed content-free completion signal; errors, cancellation, commands, replay and stale completion MUST NOT emit it. Mouse, focus, paste, alternate-screen, raw-mode and any owned title state MUST be restored or restoration attempted on every exit path.

#### Scenario: Unsafe text and cleanup

- **WHEN** session metadata contains control characters or terminal initialization, rendering or execution fails
- **THEN** it cannot inject terminal escapes and every acquired terminal state is restored through the same guard

#### Scenario: Unfocused completion

- **WHEN** the current operation successfully completes a new turn while the terminal is unfocused
- **THEN** only a fixed content-free signal is emitted, with no prompt, answer, actor-private text or raw error content
