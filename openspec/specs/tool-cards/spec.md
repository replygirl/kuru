# tool-cards Specification

## Purpose
Present admitted tool calls as bounded interactive cards with safe external output, checked recorded file diffs and truthful partial, settled or unavailable details while preserving actor privacy and runtime ownership.

## Requirements

### Requirement: Cards represent final admitted tool calls

The interactive TUI SHALL render distinct tool cards from final admitted call identity after pre-tool rewrites, with stable admission order and truthful pending, completed, denied, failed, cancelled or unavailable outcomes. A provider's provisional or terminally omitted call SHALL NOT create an admitted card or dispatch. Card identity SHALL include the initiating session, turn, actor, invocation and call; selected-speaker changes and reverse settlement SHALL NOT change ownership. Card presentation SHALL NOT repeat the final assistant answer.

#### Scenario: Same-name calls settle in reverse order
- **WHEN** two admitted calls share a tool name and the later call settles first
- **THEN** their distinct identities and admission order remain stable and only the matching card settles.

#### Scenario: Discarded provider scaffold
- **WHEN** streamed scaffolding is absent from the terminal authoritative call set
- **THEN** no admitted card or effect is invented.

### Requirement: Private bounded details and honest replay

Tool cards SHALL offer independently collapsible bounded external-tool output through a private presentation projection. Cognitive and peer operations, including `peer_send`, `relate`, `state_report`, `remember` and `a2a_send`, SHALL expose only identity and outcome metadata, never their argument or result bodies. Presentation SHALL preserve existing result projection/redaction and public three-key event, journal and TurnOutput contracts. Retained detail count, aggregate bytes, partial previews and queued updates SHALL have fixed bounds with visible omissions; no hidden card collection SHALL grow across a long session. Historical or exact replay SHALL not redispatch effects or imply unavailable transient bodies, receipt identity or admission order are known. Repeated or late updates SHALL not undo a settled state or duplicate an answer.

#### Scenario: Historical metadata has no saved body
- **WHEN** a recorded outcome is replayed without current-session detail
- **THEN** its known identity/outcome remains truthful, unavailable details are stated explicitly and no provider or tool is called.

#### Scenario: Detail window reaches capacity
- **WHEN** calls exceed the bounded retained detail window or updates are omitted
- **THEN** the user sees the omission/unavailable state, execution continues normally and retained presentation memory stays bounded.

#### Scenario: Private cognitive invocation
- **WHEN** a peer or cognitive call contains private text or state
- **THEN** neither expanded cards, arguments, partial previews nor terminal output disclose that body.

### Requirement: Practical interactive tool presentation

Cards SHALL preserve usable selection and expand/collapse controls at 80 and 120 columns, correct transcript scroll clamping and existing approval, picker, composer and compaction-notice priorities. Partial or truncated output SHALL be visibly distinguished from a settled result. Cancellation SHALL retain a truthful cancelled/incomplete presentation without accepting later preview updates as completion.

#### Scenario: Resize during a noisy admitted shell call
- **WHEN** a partial shell preview is expanded, resized and cancelled
- **THEN** both widths retain usable controls, safe partial state and the settled cancellation marker without duplicate answer or stale active preview.
