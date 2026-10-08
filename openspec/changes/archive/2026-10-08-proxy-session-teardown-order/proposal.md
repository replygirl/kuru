# Proposal

## Why

Exact-main CI after PR #263 exposed the lost-commit packet fixture waking
production reconciliation before its own session-end observation settled. The
observer belongs to the migration engine, which production may retire before
returning the replacement store.

## What Changes

- In `packages/kuru-memory/src/store/recovery_tests.rs`, retain both TCP streams
  through forwarding, intercept the selected fault, close upstream and verify
  session teardown before delivering client EOF.
- Keep observation outside the forwarding pair and include actual task failures
  in proxy shutdown diagnostics.
- Preserve ordinary half-close reply draining and verify it with a real TCP
  regression, alongside both lost-commit paths and existing absent-request cases.

## Impact

Test fixture ownership and ordering only. No production changes, relaxed
assertions, deadline changes, workflow changes, mise changes or roadmap edits.
