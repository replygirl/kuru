# Spec Delta

## MODIFIED Requirements

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
