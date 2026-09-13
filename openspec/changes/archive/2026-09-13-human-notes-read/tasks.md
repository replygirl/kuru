## 1. Runtime notes projection

- [x] 1.1 Add the public runtime `NotesView` and provider-free live-store reader, requiring a persisted current-mode topology and sharing its identity rules while deriving only the `/notes` namespace; verify distinct conversation/note markers, exact archived IDs, unknown input, missing topology and candidate rejection without seeding or mutation.
- [x] 1.2 Implement bounded N+1 note retrieval for requested limits 1 through 1000, retaining newest messages in chronological order and a truthful truncation flag; verify exact-limit and limit-plus-one boundaries in the runtime fixture.

## 2. Human command adapters

- [x] 2.1 Add `kuru memory notes <IDENTITY> --limit N` to the read-only existing-Dolt-store CLI branch, resolving saved/explicit mode through existing preferences finalization and serializing `NotesView`; verify actual binary invocation with an unavailable provider route, fresh-store non-creation and legacy-only non-import.
- [x] 2.2 Add typed TUI `/notes ID` dispatch with requested limit 100, preserving `/memory ID` as conversation-only inspection; verify the real TUI runtime dispatch returns notes metadata/content without changing the existing command result.
- [x] 2.3 Update usage/memory documentation and curated command/memory references with command syntax, current-mode and archived-ID selection, newest-note limits and truncation metadata; run documentation build/content/link checks.

## 3. Acceptance evidence

- [x] 3.1 Run the focused runtime and TUI/CLI package checks through their package-owned mise tasks and record the distinct markers, archived identity, unknown ID, N+1, provider-free, and fresh-store observations in `verification.md`.
- [x] 3.2 Run the native Windows notes cases and record their result in `verification.md`; verify the same existing-store/read-only behavior rather than treating cross-compilation as execution evidence.
