## ADDED Requirements

### Requirement: Review newly encountered instruction authority before continuation

Kuru SHALL derive a new complete, exact-root authority manifest from the existing immutable configuration snapshot and the exact checked nested instruction bytes captured for an authorized target. A matching stored approval for that complete manifest or a fresh applicable one-invocation grant SHALL be required before new bytes activate. During an interactive turn, Kuru SHALL offer the existing in-process once, persistent full-manifest, and deny choices before continuing; refusal or a closed review surface SHALL leave the new bytes inactive and prevent the pending target's effect or result exposure. A headless turn without a matching approval or explicit applicable one-invocation grant SHALL return a bounded trust-required result with a concrete review remedy and no unreviewed instruction injection. Tool permission approval and workspace trust SHALL remain separate decisions. A persistent nested approval SHALL preserve root-only startup approval, bind the exact active nested source set and complete extended manifest, and be removed by ordinary `trust revoke` without scanning unrelated project paths at startup.

#### Scenario: Interactive new nested claim
- **WHEN** an authorized actor read first reaches a nested source absent from the currently approved manifest
- **THEN** the foreground review shows the changed complete manifest without source contents and no target result or next provider request is released until the user grants applicable trust.

#### Scenario: Denied trust or closed review
- **WHEN** the user refuses nested instruction trust or the foreground channel closes
- **THEN** Kuru neither activates the new source nor executes a pending mutation or exposes the pending read result.

#### Scenario: Headless path discovery
- **WHEN** a headless actor call reaches an unapproved nested source
- **THEN** Kuru returns a bounded trust-required diagnostic and leaves the source and target result inactive, without waiting for interactive input.

#### Scenario: Persistent nested approval on another launch
- **WHEN** the user persistently approves one path-qualified complete nested manifest and starts a later turn at the same root
- **THEN** the root-only preflight remains valid, only a call entering that checked path set can use the nested approval, and changed source bytes or identity require fresh review.

#### Scenario: Revocation and concurrent approval
- **WHEN** `trust revoke` removes an existing workspace record while another session is reviewing a nested persistent choice against that record
- **THEN** the stale reviewer cannot silently recreate the revoked approval or drop another session's newer grant; it must review the current state again.

#### Scenario: Permission denial precedes discovery
- **WHEN** tool permission denies a target beneath a nested instruction directory
- **THEN** that target does not cause the nested source to be captured, reviewed, or activated.
