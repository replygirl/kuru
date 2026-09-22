# verified-file-edits Specification

## Purpose
Define how Kuru applies bounded native file edits through checked project authority,
records private durable before/after checkpoints, and permits selected undo only
when the current target still matches the settled effect. Uncertain publication
remains explicit and cannot authorize a blind retry or undo.

## Requirements

### Requirement: Exact-context file edits

Kuru SHALL offer a native `file_edit` tool for bounded regular files below the checked project root. Its ordered hunks MUST match the captured source bytes and context uniquely without overlap; a stale, ambiguous, malformed or oversized edit MUST fail before any target mutation. Kuru MUST recheck the expected source bytes and checked target identity immediately before publication and MUST NOT guess a different hunk location or partially publish a multi-hunk edit.

#### Scenario: Ambiguous second hunk
- **WHEN** a two-hunk edit has one valid hunk and one ambiguous or stale context
- **THEN** the original file remains byte-identical and no checkpoint is marked applied.

#### Scenario: User changes the target before publication
- **WHEN** the captured source or checked target identity changes before an edit is published
- **THEN** the edit reports a conflict and does not overwrite the user's file.

### Requirement: Private bounded mutation checkpoints

Every admitted Kuru-managed `file_write`, `file_edit` and `file_delete` SHALL reserve capacity and durably record a project-bound recovery receipt with bounded before/after snapshots before attempting target mutation. The receipt MUST bind the canonical project, target, operation/turn/call identity and expected source/post state; it MUST reside in an owner-private directory outside the tool root and MUST NOT become readable through native tools. Individual snapshots and total retained checkpoint storage SHALL have documented fixed bounds. Capacity exhaustion MUST refuse the promised mutation before it changes the target. No checkpoint SHALL expire or be pruned automatically.

#### Scenario: Capacity is full
- **WHEN** the next write would exceed an individual or project checkpoint bound
- **THEN** Kuru refuses the write with the existing target unchanged and an actionable capacity result.

#### Scenario: Staged bytes and final receipt size
- **WHEN** a replacement stages private bytes or a create needs a final post-publication identity
- **THEN** Kuru reserves the retained stage and worst-case settled receipt size before target publication, and keeps staged payload bytes behind a checked owner-private directory until checked publication.

#### Scenario: Windows access-policy handoff
- **WHEN** a staged file is published on Windows with an ordinary inherited or protected source access policy
- **THEN** its effective DACL is shielded during staging and its source inheritance behavior is finalized on the retained published handle before the checkpoint is marked applied; a failure after publication is reported as uncertain.

#### Scenario: Source access policy changes during publication
- **WHEN** the held source file's owner, group, mode or DACL changes after Kuru captures the staged access policy
- **THEN** a detected change before publication refuses the effect, and a detected change after publication retains an uncertain receipt rather than claiming applied; Kuru does not claim to lock out arbitrary external access-policy writers.

#### Scenario: Create, replace and delete
- **WHEN** Kuru successfully creates, replaces or deletes a checked file
- **THEN** a durable receipt retains enough exact before/after evidence for selected undo after restart.

### Requirement: Exact-effect reconciliation and selected undo

Kuru SHALL reconcile an interrupted file mutation by its retained receipt and checked target state before any exact retry. It MUST NOT blindly replay a file effect, infer that an absent target alone proves an uncertain delete, or infer that matching bytes or a serialized file identity after restart prove a prepared create or replacement was published by Kuru. An unresolved receipt MUST NOT authorize undo. A durably applied receipt SHALL remain eligible for selected undo after restart: the undo handler SHALL reopen the current target through checked handles, pass normal project and file permission/protected-path checks, require the current target to match the recorded post-edit state, and restore only that selected Kuru edit. The original process's publication handle is not required for this separately checked undo. A current target with different checked identity or bytes MUST produce a conflict without being overwritten. Undo SHALL retain its own durable result so interruption and exact retry do not reverse it twice. File undo is distinct from dream undo and conversation rewind; shell and MCP effects remain outside this guarantee.

#### Scenario: Exact retry after an accepted write loses its reply
- **WHEN** a retried operation finds its durably applied original receipt and exact post-publication target
- **THEN** it reports the settled original result without applying the content a second time.

#### Scenario: Later user edit
- **WHEN** a user changes a file after a selected Kuru edit so its checked identity or bytes differ from the recorded post-state
- **THEN** undo refuses to replace those bytes and leaves both the user file and recovery receipt intact.

#### Scenario: Uncertain delete
- **WHEN** a delete reply is lost and the target is absent without proof of Kuru's publication
- **THEN** the receipt remains explicitly unresolved and automatic retry or undo does not restore the old bytes.

#### Scenario: Uncertain create or replacement after restart
- **WHEN** a process restarts with only a prepared create/replacement receipt and the target happens to match its expected post bytes
- **THEN** Kuru reports the receipt unresolved and does not treat that content match as proof of Kuru's publication or permission to undo.

### Requirement: Explicit checkpoint inspection and pruning

Kuru SHALL provide bounded inspection of project checkpoint IDs, target paths, operation identity and settled/uncertain status without exposing snapshot bodies by default. Ordinary pruning SHALL remove only selected settled records after their private identity and absence of an in-flight operation are checked. A separate explicit discard of one inactive uncertain receipt MAY forfeit its recovery evidence; the user-visible result MUST say that undo for that receipt is no longer available. Pruning SHALL NOT remove an active operation or silently erase uncertainty.

#### Scenario: Explicit uncertain discard
- **WHEN** an inactive uncertain receipt cannot be conclusively reconciled and the user explicitly requests its discard
- **THEN** Kuru removes only that selected receipt and its exactly identified private staging artifacts, reports the lost recovery capability, and does not mutate the project file; a replaced stage or template is never selected by name alone.

#### Scenario: Staging parent was replaced
- **WHEN** explicit discard cannot revalidate the captured parent or a staged-object identity
- **THEN** it refuses that discard with a precise conflict and retains the receipt rather than deleting a different object or leaving untracked private bytes.
