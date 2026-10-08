# Proposal

## Why

Both native Windows architectures show the newly added fixture feeding Unix
bracketed-paste bytes into crossterm's native key-event backend. That backend
never emits a paste event, so the fixture does not establish its intended proof.

## What Changes

- Exercise recall/search, saved Unicode drafts, native editing and long literal
  key-input history through `tests/support/windows_recall.rs`, preserving exact
  completed durable prompt checks at both practical terminal widths.
- Keep paste-chip/rejection checks in the existing Unix PTY and direct-event
  tests, and document the unverified Windows atomic-paste limitation accurately.

## Impact

Only the new native fixture and owning documentation change. No test is ignored,
no production input backend is added, and native CI must prove the corrected
recall contract. The source inventory, 95% gate and dependency pins stay fixed.
