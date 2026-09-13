## 1. Explicit selected-note forgetting [critical]

- [x] 1.1 @integration (agent) use an isolated real Dolt store to delete one selected current `/notes` sequence -> only that row is absent from the active notes view, a new revision records the operation, and the prior revision retains the row.
- [x] 1.2 @e2e (agent) invoke `kuru memory forget ID --note SEQUENCE` against an isolated existing store with provider credentials unavailable -> the command returns the resolved identity, selected sequence, and retained-history disclosure without starting a provider, tool, or conversation.
- [x] 1.3 @regression (agent) target a current dream-authored notes row -> the exact dream row can be selected and removed while another note and every transcript row remain unchanged.

## 2. Selection and user-visible identifiers

- [x] 2.1 @integration (agent) read selected notes for active and archived exact identities -> chronological rows include stable signed sequences and original roles, while unknown or stale sequences reject without mutation.
- [x] 2.2 @runtime (agent) dispatch `/notes ID` through the existing TUI command path -> rendered JSON includes the same sequenced notes view and does not change `/memory` conversation inspection.

## 3. Native portability

- [ ] 3.1 @runtime (agent) run the selected-note CLI/store behavior on native Windows -> current-row deletion and retained-history disclosure pass under the native Dolt supervisor.

## Observed evidence

- `CARGO_TARGET_DIR=/private/tmp/kuru-phase0-note-controls/target mise run //packages/kuru-memory:typecheck` exited 0; `/private/tmp/kuru-note-controls-memory-typecheck-final.log`.
- `CARGO_TARGET_DIR=/private/tmp/kuru-phase0-note-controls/target mise run //packages/kuru-runtime:typecheck` exited 0; `/private/tmp/kuru-note-controls-runtime-typecheck.log`.
- `CARGO_TARGET_DIR=/private/tmp/kuru-phase0-note-controls/target mise run //apps/kuru-tui:typecheck` exited 0; `/private/tmp/kuru-note-controls-tui-typecheck-final.log`.
- The focused real-Dolt selected-row test and signed legacy sequence test each passed one test; `/private/tmp/kuru-note-controls-memory-selected-test.log` and `/private/tmp/kuru-note-controls-memory-signed-sequence-test.log`.
- The focused runtime dream-role deletion, provider-free CLI/reopen, and TUI `/notes` rendering tests each passed one test; `/private/tmp/kuru-note-controls-runtime-forget-test.log`, `/private/tmp/kuru-note-controls-tui-forget-cli-test.log`, and `/private/tmp/kuru-note-controls-tui-notes-render-test.log`.
- `mise run //apps/kuru-docs:format:check` exited 0 after the package formatter fixed the edited markdown.
- Native Windows selected-note behavior remains required and unrun; no non-Windows build or fixture is treated as native evidence.
