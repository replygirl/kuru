# Proposal

## Why

In the interactive TUI the first-run memory notice is pushed as a local `system` transcript item, and every newest-page install (`/` submission calls `follow()`, completed turns and session restore) drains all local items and re-appends them after the incoming public records. The notice, and any other local item such as `/help` output, `kuru` notices or error lines, therefore jumps to sit directly above the latest user message after every turn. The notice also repeats a paragraph of CLI reference inside the conversation, which the maintainer does not want shown as a transcript message at all.

## What Changes

- Each local transcript item records a stable anchor: the identity of the last item with a persisted (non-`Local`) identity before it, or none when it preceded every record. Page installs re-place retained locals immediately after their anchor, before public records when the anchor is none, before the next surviving later item when the anchor is absent from the new window, and at the end otherwise. Public-page arithmetic (eviction, newer insertion, cursor lookup, search) counts only public items, so interleaved locals cannot corrupt it.
- The first-run memory notice becomes a status-line indicator `Memory ready · kept until removed · /status · "<directory>"`. It does not expire on the transient-notice timer, clears on the first key or paste, takes precedence over transient notices, and is still recorded only after the frame containing it completed and before input is admitted. It is never a transcript item, persisted message or provider input.
- `/status` gains a persistent `Memory: "<directory>" · kept until removed · kuru memory --help` line.
- The notes/export/forget/purge reference moves to `kuru memory --help` and the docs. Headless runtime commands keep their once-per-project stderr line, shortened to the directory, the retention sentence and a `kuru memory --help` pointer.

## Capabilities

### New Capabilities

### Modified Capabilities

- `chat-harness`: the interactive first-run notice is a status-line indicator plus a `/status` line instead of a system transcript entry listing every control.
- `interactive-transcript-navigation`: local display items keep their position relative to the public records they followed across page installs.

## Impact

`apps/kuru-tui/src/ui/transcript.rs`, `apps/kuru-tui/src/ui.rs`, `apps/kuru-tui/src/ui/render.rs`, `apps/kuru-tui/src/memory_notice.rs`, `apps/kuru-tui/src/cli.rs` (memory long help) and their tests; `docs/usage.md`, `docs/memory.md`, `apps/kuru-docs` memory and command pages. No memory-package, marker-format, provider or runtime change; the per-project version marker and its recording boundary are unchanged.

## Surfaces

- [x] interactive — TUI transcript ordering, status line, `/status`, `kuru memory --help`, headless stderr notice
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
