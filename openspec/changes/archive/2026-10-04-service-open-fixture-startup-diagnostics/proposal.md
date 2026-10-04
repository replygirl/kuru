# Proposal

## Why

A direct `ServiceOwner::open` fixture can fail during Dolt startup without including the bounded private server-log tail already provided by test support. The current CI failure therefore exposes only a generic readiness error, leaving the actual startup cause unavailable.

## What Changes

- In `packages/kuru-memory/src/service.rs`, annotate the targeted direct owner-open fixture error with the existing `fixture_startup_error` helper before its temporary fixture is dropped.
- Add a controlled service-owner startup-path regression whose configured executable delegates version verification to the actual Dolt binary, then emits a fake diagnostic and exits during startup. Compare the unannotated error with the helper-annotated error, checking the original startup cause is retained and the bounded log tail is added. Keep the change test-only.

## Impact

Only `kuru-memory` tests and this change's Cospec artifacts are affected. The regression runs one owner startup attempt through the real supervisor: its configured executable delegates version validation to the real Dolt binary, then emits a controlled fake startup failure. It does not change runtime behavior or diagnose the hosted exit cause.
