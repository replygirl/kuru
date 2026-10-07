# Proposal

## Why

The corruption fixture intentionally retains failed native validation stages;
Windows CI rejected the corrupt image but teardown found its nested stats
repository at depth nine, beyond the fixture's default eight-level scan.

## What Changes

Name a nine-level scan at this fixture's existing guarded root, accounting for
state/backup-validation/stage/data/kuru/.dolt/stats/.dolt/noms or temptf. Retain
every corruption, inventory, source-image and awaited-quiescence assertion.

## Impact

Only the root setup in `packages/kuru-memory/src/backup.rs` changes. Global
depth eight, entry budget 4096, strict guard, deadlines and production stay
unchanged. Acceptance is the existing corrupt-image case, backup closing guard
and named-depth guard, plus relevant static checks; native Windows remains
required in fresh full CI.
