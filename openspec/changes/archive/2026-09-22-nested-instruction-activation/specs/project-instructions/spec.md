## ADDED Requirements

### Requirement: Path-qualified nested instruction activation

After a checked native file or search target is admitted for a prompt-bearing actor turn, Kuru SHALL discover `AGENTS.md` and `CLAUDE.md` in directories strictly below the invocation root and at or above that target's containing directory. It SHALL compose those sources in outermost-to-most-local directory order with the already captured ancestor and root sources, applying the existing import confinement, checked identity, deduplication, cycle, file, aggregate, graph, depth, and notice rules across the complete active set. A source in a sibling or unrelated subtree MUST NOT become active merely because another path was used. Kuru SHALL retain the captured bytes and source identities for the turn and SHALL use the reviewed projection for the next actor inference; it MUST NOT reread prior captured sources to reinterpret an approval.

#### Scenario: File use enters one nested subtree
- **WHEN** an authorized actor file call targets `src/lib.rs` and `src/AGENTS.md` exists
- **THEN** Kuru captures and reviews that nested source before it can appear in the next prompt, while `tests/AGENTS.md` remains inactive.

#### Scenario: Nested import and global cap
- **WHEN** a newly encountered nested source imports Markdown and an import would exceed an existing global instruction cap
- **THEN** Kuru keeps the complete fitting sources, omits the over-cap branch wholly, and exposes a bounded truthful notice before inference and in the prompt.

#### Scenario: Prior source changes after capture
- **WHEN** a root or previously activated nested instruction source changes after its bytes were captured for this invocation
- **THEN** the running actor continues with only the captured reviewed bytes, while a later invocation sees a changed authority manifest.

### Requirement: Replan after newly encountered instructions

When an authorized mutating file call first encounters applicable nested instructions, Kuru SHALL settle that original call as requiring a new actor plan before any file effect and SHALL NOT replay it automatically. The next inference SHALL include the newly reviewed instruction projection and a bounded replan result tied to the original call. Read-only calls MAY continue after trust review, but their path or content results MUST NOT be exposed before applicable review succeeds.

#### Scenario: New instruction precedes a planned write
- **WHEN** an actor plans `file_write` under a newly encountered nested `CLAUDE.md`
- **THEN** the write does not occur, the original call receives one replan-required result, and the next actor inference includes the reviewed nested source.

#### Scenario: Already active instruction
- **WHEN** the target's applicable nested instructions are already active in the running actor context
- **THEN** ordinary permission and file-effect behavior proceeds without a redundant replan.
