## ADDED Requirements

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
