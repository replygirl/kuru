# Proposal

## Why

Actual Windows CI completes all seven recall turns but fails exact persisted prompt equality without showing the mismatched value. Report the synthetic expected and actual prompt for the failing turn so transport and composer defects can be distinguished.

## What Changes

- Keep `tests/support/windows_recall.rs` strict full-string equality for seven completed turns at both terminal widths; include the failing turn, byte lengths and Unicode scalars in its diagnostic.

## Impact

Only the isolated native test diagnostic changes. No production behavior, accepted text, timeout, skip or coverage policy changes.
