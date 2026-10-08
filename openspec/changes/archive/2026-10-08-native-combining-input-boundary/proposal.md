# Proposal

## Why

Actual Windows x64 and ARM recall transcripts both lose U+0301 from the synthetic decomposed draft. Observe native input events separately from composer and recall before attributing or correcting the loss.

## What Changes

- Add an isolated `input-events` mode to the existing Windows terminal fixture, entering the same public terminal session and reading actual crossterm events through the same ConPTY byte input.
- Extend `tests/support/windows_recall.rs` with a causal probe that records character scalars, modifiers and event kinds, including releases, and checks literal byte input before any composer or recall logic.

## Impact

Only native test fixtures change. Preserve exact seven-turn transcript assertions, existing bounded waits, native mode restoration and owned cleanup. Do not add an input backend, injected key substitute, production tracing or relaxed text acceptance.
