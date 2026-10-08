# Proposal

## Why

Native Windows coverage does not exercise failure to remove a descendant directory after its payload has already been deleted. That partial effect must retain the exact uncertain outcome and descendant identity instead of authorizing a fresh retry.

## What Changes

- Extend `packages/kuru-platform/src/fs/windows.rs` tests with a real pinned child directory containing one payload: delete the payload, refuse final child removal through native sharing, and verify exact identities, ACLs, descendant reporting and checked cleanup.

## Impact

One native filesystem test only; no production behavior, kernel substitution, workflow, wait-bound or coverage-policy changes. Native CI acceptance remains required.
