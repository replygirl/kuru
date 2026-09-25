## ADDED Requirements

### Requirement: Working session commands share the registry

When the P11 backend is present, the TUI SHALL define `/new`, `/sessions`, `/resume` and `/export` in the existing typed command registry and SHALL route them to the same typed session catalog, picker, resume and export operations used by the CLI. Help, completion, argument validation and dispatch identity MUST agree; session lifecycle commands MUST NOT become provider turns or bypass reconciliation required by a mutation.

#### Scenario: Session command is discoverable and local
- **WHEN** a user completes and submits a registered session command
- **THEN** help and completion show its canonical usage, the corresponding local lifecycle handler runs, and no provider request is made.

#### Scenario: Failed session command preserves the surface
- **WHEN** a session command names a pending, removed, missing, malformed or uncertain target
- **THEN** the typed diagnostic is shown without changing the current session, draft, transcript or permission state.
