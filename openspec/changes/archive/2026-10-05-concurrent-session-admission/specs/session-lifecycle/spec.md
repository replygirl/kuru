# Spec Delta

## MODIFIED Requirements

### Requirement: Deterministic resume and continue

Exact resume SHALL accept only a stored nonremoved session in the canonical project and MUST acquire its checked driver claim before actor/provider/tool work. `--continue` SHALL select the most recently updated nonremoved session by durable catalog order, with a stable identity tie-break, and SHALL fail clearly when none exists. A busy selected session MUST produce an actionable refusal rather than silently select another session. Every ordinary new process invocation without an explicit selector MUST create a fresh session. Every new process invocation, including resume, continue and a forked session, MUST start a new permission-grant session and MUST NOT inherit session-only grants. Distinct sessions MAY be driven concurrently; a second surface for the same driven session MUST be refused.

#### Scenario: Continue chooses the durable latest session
- **WHEN** multiple stored sessions exist, including a newer removed session
- **THEN** continue deterministically selects the most recently updated ordinary session and does not restore or select the removed session.

#### Scenario: Resume does not inherit authority
- **WHEN** a fresh process resumes or continues a session whose previous process held session-only tool permission
- **THEN** the transcript and session mode resume after checked claim acceptance but the permission requires fresh admission under the new process.

#### Scenario: Busy explicit selector
- **WHEN** resume or continue selects a session driven by another process
- **THEN** it refuses before private actor context or provider dispatch, preserves the deterministic selected identity in its diagnostic, and does not start a silent replacement session.

### Requirement: Consistent CLI and TUI session management

CLI session commands and the TUI picker SHALL use the same typed catalog, live-presence and mutation results for list, rename, remove, restore, fork, resume, continue and export. The picker MUST distinguish ordinary and removed sessions, expose settled fork boundaries, distinguish known live presence from unknown presence, and provide actionable driven, changed, pending, missing, removed and uncertain diagnostics without making a provider request. Existing CLI listing array shape MUST remain compatible while safe per-row presence metadata is added. Cancelling the picker's inline rename editor before submission MUST restore the composer input saved when that editor opened.

#### Scenario: Picker and CLI observe the same lifecycle
- **WHEN** a session is renamed, removed, restored or forked through either surface and the other surface reopens the project
- **THEN** both show the same identity, label, status, provenance and transcript and neither lifecycle action invokes a provider.

#### Scenario: Independent ordinary conversations
- **WHEN** two ordinary invocations start from the same project without resume selectors
- **THEN** both are admitted to distinct fresh sessions on one memory owner, each surface identifies its own selection, and a second surface requesting either active session is refused.

#### Scenario: P30 admission remains absent
- **WHEN** a historical P11 caller without validated session claims attempts another conversation while the project has an active driver
- **THEN** ordinary inspection remains available where its compatible read-only authority permits it, but that caller cannot infer a live-session claim, admit an unclaimed second driver or attach a second surface to the active session.

## ADDED Requirements

### Requirement: Atomic checked selection and claimed lifecycle exclusion

Session selection SHALL prepare and validate its target without changing the selected session, grant context or admitted actor snapshot. The owner's final catalog check and claim switch MUST occur atomically under its existing write guard, checking the exact old claim and captured target catalog. A typed refusal MUST preserve usable old selection and ownership. Uncertain or accepted-but-unpublished selection MUST fence driver work and remain explicitly pending until exact recovery or checked restoration; it MUST NOT masquerade as unchanged usable selection. New/fork failure MAY retain an honest durable unselected catalog row, but local selection MUST publish only after required persistence and ownership are proved. Native old-session exclusion MUST remain held while target preparation or checked restoration is unresolved.

Catalog lifecycle mutations MUST consult driver ownership under that same owner guard in addition to their existing generation/receipt checks. Rename and fork MUST refuse a different active driver; removal MUST refuse any driven session. Frontend current-session checks alone MUST NOT enforce this rule. A driver's own allowed rename/fork MUST retain existing checked lifecycle and private-history semantics.

#### Scenario: Catalog changes while selection is staged
- **WHEN** a target generation, lifecycle or retained catalog tuple changes after it was prepared
- **THEN** final owner selection refuses without releasing the old claim, resetting permissions, publishing target context or dispatching a provider.

#### Scenario: New or fork selection fails
- **WHEN** catalog creation or fork commits but later target validation, selection or persistence fails
- **THEN** the durable row is reported honestly if retained, old selection remains usable only with proved restored ownership, and unresolved accepted selection stays fenced rather than silently publishing a new session.

#### Scenario: Lifecycle command races a live driver
- **WHEN** another caller attempts rename, fork or removal against a driven session through the managed memory API
- **THEN** the owner refuses the unauthorized mutation before effects or receipt, even if the caller bypasses the TUI, and the driver keeps its exact session and history.
