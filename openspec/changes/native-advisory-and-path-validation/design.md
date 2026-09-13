## Context

The delivery scanner reads a clone-local Git configuration as untrusted input
and accepts an enumerated fresh-clone inventory. The Windows native fixture
creates that clone with stock Git for Windows, which adds `core.symlinks=false`.
The current inventory does not recognize that key. Separately, Windows
canonicalization adds a verbatim path prefix, while core provenance intentionally
uses `char::escape_default` for its safe display value.

## Goals / Non-Goals

**Goals:**

- Accept the one documented stock Windows clone value without weakening the
  clone-local configuration allowlist.
- Make tests compare native paths and the existing display contract correctly.

**Non-Goals:**

- Change scanner command construction, clone ownership, production path
  representation, or public configuration provenance.
- Accept arbitrary `core.*` settings or alter a caller-supplied repository.

## Decisions

- Treat `core.symlinks` as an optional accepted key only when its value is
  exactly `false`. A generic boolean allowance would accept a non-stock setting
  and weaken the local-config boundary.
- Canonicalize both test paths before comparing identity. This preserves source
  behavior and tolerates Windows' canonical verbatim prefix.
- Derive the test expectation for provenance using the same escaped display
  transformation as the established contract, rather than requiring Unix path
  separators.

## Integration contract

Git local configuration remains an untrusted integration boundary. The accepted
Windows-specific bookkeeping entry is `core.symlinks=false`; all unknown keys
and `core.symlinks=true` remain rejected.

## Risks / Trade-offs

- [A Git build emits additional clone-local keys] → the validator continues to
  reject them until a focused, reviewed compatibility decision adds an exact
  value.
- [Native path spelling differs] → tests compare canonical paths, while the
  public safe display continues to show the platform's escaped canonical text.
