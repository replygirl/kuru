## Why

A native Windows shell fixture timed out before its first user-command marker, leaving startup and stock module import stages indistinguishable. A bounded fixture-only trace will identify the last completed stage without changing shell behavior or deadlines.

## What Changes

- Add static startup markers to the existing isolated Windows shell environment fixture in `packages/kuru-connectors/src/tools.rs`, gated by its existing child/root opt-in.
- Assert the complete ordered trace on success and report its existing bounded partial trace on failure.

## Impact

Only the Windows connector test fixture gains trace writes; ordinary shell requests and non-test builds retain their emitted source bytes. No additional CI job or test runtime is planned.
