# Tasks

Acceptance evidence is recorded against each task as its check finishes; unrun checks are named with the reason. Affected surfaces: `packages/kuru-delivery` test constants and support (listed in the proposal) and `docs/development.md`.

## 1. Derivation record

- [ ] 1.1 Identify, for each of the four 180 s constants, the product or vendor budget it depends on (or the event it should wait for), citing the source budget in code or vendor documentation, and verify the record names a source for every constant and invents no number.
- [ ] 1.2 Write the derivation record beside the constants and verify each constant cites it.

## 2. Constants and call sites

- [ ] 2.1 Derive or replace `TIMEOUT` in `tests/bootstrap_windows.rs` and verify its call sites still assert the same outcomes.
- [ ] 2.2 Derive or replace `DEADLINE` in `tests/support/mise_acceptance.rs` and `tests/support/previous_updater.rs`, exposing an existing product budget to test support only if needed, and verify with `mise run //packages/kuru-delivery:test`.
- [ ] 2.3 Derive or replace `WRAPPER_LAUNCH_BUDGET` in `tests/powershell_diagnostics.rs` and verify no remaining literal lacks a derivation by grepping the package tests for `from_secs(180)`.
- [ ] 2.4 Update the pin test in `tests/powershell_diagnostics.rs` to pin the derivation and the shard deadline bound, and verify it passes and fails when a constant is changed without its derivation.

## 3. Documentation

- [ ] 3.1 Change the `docs/development.md` open-time line to say exit is stamped after both pipes close and the command is reaped, and verify against the exit-stamp code from commit 5030b4bc.
- [ ] 3.2 Verify `mise run docs:check` passes.

## 4. Evidence

- [ ] 4.1 Run lint, format check and the delivery package tests, record the observed results (Windows-only tests that cannot run on this host are named as unrun, with CI as their evidence), and verify the hk pre-push hook passes.
