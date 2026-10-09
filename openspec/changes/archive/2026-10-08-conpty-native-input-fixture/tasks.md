# Tasks

## 1. Native input acceptance

- [x] 1.1 Correct `apps/kuru-tui/tests/support/windows_recall.rs` to drive actual native key events for recall/search, saved Unicode drafts, middle-of-draft editing and long literal history. Preserve seven exact completed durable prompt checks at 120/80 columns, explicit fixture cleanup, and unchanged wait bounds; do not inject or claim a native paste event.
- [x] 1.2 Document the Windows atomic-paste transport limitation in owning composer docs and the working doc, preserving the existing Unix PTY/direct-event paste coverage. Run formatting, affected Windows-target lint and public docs checks; identify native behavior still pending CI and archive before the follow-up commit.

Final source passes TUI Windows-target lint, repository formatting and the full
public documentation build/content/link checks. Independent review verified
the actual physical-row cursor predicates at both widths, the nine-byte middle
edit and all seven exact completed JSONL prompts, including the decomposed
draft. Both prior native architectures failed at the unsupported paste-event
assumption; corrected native execution remains pending PR CI. No input backend,
timer, ignored test, coverage exclusion or accepted-Paste spec is changed.
