# Tasks

## 1. Composer primitives

- [x] 1.1 Add bounded session-keyed submitted history, Ctrl-R matching, grapheme/line motion helpers, and paste-chip range/projection operations in `ui/composer.rs`; verify their limits and Unicode/range invariants with focused unit cases.
- [x] 1.2 Keep paste acceptance atomic at the 128 KiB draft limit and chip projection separate from canonical text; verify rejected input leaves draft and metadata unchanged and chip actions edit only their exact literal span.

## 2. View and event integration

- [x] 2.1 Integrate grapheme editing, line/vertical motion, recall/search and chip controls with current `View` state; verify exact canonical text and cursor byte boundaries in focused View tests.
- [x] 2.2 Record dispatched prompts against the session captured at dispatch and preserve unsent draft/cursor/chips across recall, picker, rename, instruction and approval state changes; verify cross-session settlement and cancellation fixtures.
- [x] 2.3 Preserve current `/config`, command completion, cancellation, picker and approval routing priority while adding composer shortcuts; verify existing routing cases alongside disabled approval choices and ordinary digits after dismissal.

## 3. Rendering and bounded scroll

- [x] 3.1 Render grapheme-aware draft and chip projection from the canonical draft, with cursor placement derived from the same projected layout; verify representative combining/ZWJ and width cases with deterministic TestBackend frames.
- [x] 3.2 Clamp transcript and composer view offsets to current content and viewport dimensions after edits, content changes and resize; verify state/render bounds across shrinking and growing viewport fixtures.

## 4. Integration and user guidance

- [x] 4.1 Exercise exact prompt submission, bounded session history, chips, resize and modal ownership through synchronized real PTYs and deterministic provider fixtures; verify submitted bytes and completed-frame cursor visibility without claiming live-provider behavior.
- [x] 4.2 Document composer shortcuts, ephemeral history, paste-chip behavior and the draft size limit in the command and first-conversation guides; verify owning docs build, content, and local-link checks.

## 5. Acceptance and archive

- [x] 5.1 Complete the verification ledger with observed focused tests, real PTY behavior, strict Cospec/apply results, scoped formatting/lint/docs checks, and archive readiness; retain normal archive and commit hooks for the required pre-publication workflow.
