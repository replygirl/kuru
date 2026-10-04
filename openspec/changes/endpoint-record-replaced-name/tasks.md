# Tasks

## 1. Typed replaced-name outcome

- [ ] 1.1 In `packages/kuru-platform/src/fs.rs`, make `Directory::verify` return a typed replaced-name outcome distinct from every denial, with the same kind and message for `?` propagation, and verify with a unit test that a replaced name is typed and a privacy failure is not. Evidence to collect: the test's name and result, and that the same test fails before the change.
- [ ] 1.2 Carry the outcome through `files::read_held_then` in `packages/kuru-memory/src/files.rs`, and verify existing `verify` callers in kuru-connectors and kuru-memory compile and behave unchanged. Evidence to collect: the package tests that exercise those callers.

## 2. Endpoint record reader

- [ ] 2.1 In `packages/kuru-memory/src/service.rs`, map only the typed outcome to `Ok(None)` in `EndpointRecord::read_then`, with no retry, wait or literal. Verify by reading the diff for none of those.
- [ ] 2.2 Regression test through the `between` hook: replace the record with a private successor record, assert `Ok(None)`, then assert the next read returns the new record. Evidence to collect: the test fails before the fix with `read private memory service endpoint` and passes after.
- [ ] 2.3 Tests through the same hook: replace the record with a non-private object and assert the denial still raises, and assert a privacy denial still raises. Evidence to collect: both results.
- [ ] 2.4 Run `service::tests::crashed_owner_retains_accepted_receipt_after_sibling_write` repeatedly on macOS and record the observed pass count. Name any unrun native check (Windows, Ubuntu) and the reason.

## 3. Documentation and gate

- [ ] 3.1 Add one sentence to `docs/memory.md` beside the retiring-service wait paragraph: a record replaced by a successor during a read is a miss that the next readiness poll re-reads. Verify with `mise run docs:check`.
- [ ] 3.2 Run `mise run format:code ::: lint:rust ::: lint:windows ::: typecheck ::: cospec:validate ::: cospec:managed:check ::: docs:check` and the affected package tests, recording observed results; coverage runs in CI. Name unrun checks and reasons.
