# project-instructions Specification

## Purpose
Define how Kuru captures, orders, bounds, and reviews project instruction files so the effective prompt contains only complete checked sources and reports omitted material truthfully.

## Requirements

### Requirement: Ordered checked instruction composition

Kuru SHALL capture applicable ancestor and project-root `AGENTS.md` and `CLAUDE.md` sources in outermost-to-most-local directory order, with `AGENTS.md` before `CLAUDE.md` in the same directory. It SHALL expand a standalone relative `@path` Markdown import at its directive position from the importing file's directory, confined to the original top-level source directory. It MUST read only checked regular files through checked directory identities, reject links and path escapes, detect import cycles, and render each physical source at most once. Imported bytes SHALL retain repository-origin prompt authority and SHALL NOT gain caller-local authority from a filename or import spelling.

#### Scenario: Claude wrapper imports the sibling Agents file
- **WHEN** an applicable `AGENTS.md` exists and its sibling `CLAUDE.md` contains `@AGENTS.md`
- **THEN** Kuru renders the `AGENTS.md` content once, removes the resolved import directive from the prompt, and renders the remaining `CLAUDE.md` content in deterministic order.

#### Scenario: Nested relative import and cycle
- **WHEN** a top-level instruction imports a Markdown file in its allowed subtree and that file imports an ancestor already on the active import stack
- **THEN** Kuru renders each captured source at most once, reports the closing cycle edge, and does not recurse indefinitely.

#### Scenario: Link or path escape
- **WHEN** an import names a linked source or resolves outside its top-level source directory
- **THEN** instruction capture rejects that import before prompt activation with a bounded source-category diagnostic and does not read the escaped target.

### Requirement: Graceful bounded instruction capture

Kuru SHALL bound each instruction source to 256 KiB, all active captured instruction content to 1 MiB, the distinct source graph to 128 files, and import depth to eight edges. When a source or branch exceeds one of those caps, Kuru SHALL omit it wholly, continue with other usable sources, and report a bounded truthful omission notice both to the user before inference and in the effective prompt. It MUST NOT silently truncate omitted bytes or describe them as active instructions. Duplicate imports SHALL be skipped without consuming the active content budget a second time.

#### Scenario: One oversized source among usable instructions
- **WHEN** one imported source exceeds the per-file cap and another applicable source fits
- **THEN** the oversized source contributes no instruction bytes, its omission is visible, and Kuru may continue using the complete captured bytes of the fitting source after ordinary trust review.

#### Scenario: Aggregate or graph cap reached
- **WHEN** another source would exceed the aggregate byte, distinct-file or depth cap
- **THEN** Kuru does not partially inject that source or traverse that branch, reports the cap and omitted source or branch safely, and continues with already captured material.

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
