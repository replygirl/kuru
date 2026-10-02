# Proposal

## Why

Since PR #151, `native_mise_github_backend_installs_and_activates_real_offline_kuru`
(`apps/kuru-tui/tests/windows_mise.rs` via `packages/kuru-delivery/tests/support/mise_acceptance.rs`)
fails on Windows coverage partitions with "memory service readiness deadline
exceeded" (4 hits / 37 runs measured post-#151 across all branches, 2/12 on
`main`; up from 1/20 `main` runs pre-#151): its
single timed first launch now extracts and probes the engine, builds the
per-machine store template through the full migration chain (three engine
starts), and creates the project, all under the installed binary's unchanged
30 s client wait. This is a CI flake in our own test fixture, not a product
defect, and it is fixed by warming the fixture's engine cache and store
template *before* the timed launch, the same way the existing `prefetch`
binary warms the shared test cache for ordinary unit tests.

## What Changes

- `packages/kuru-memory/src/test_support.rs`: add `template_key()`,
  `store_template_key(data, scope)`, `engine_warm_up_bound()`, and
  `TemplateCacheReceipt` (`snapshot`/`verify_used`/`Display`) — visibility-only
  additions around existing warm-up machinery, no behavior change to any
  existing caller.
- `packages/kuru-delivery/tests/support/mise_acceptance.rs`: inside
  `Installation::conversation`'s result block, after the unchanged cold
  assertion and cleanup arming, provision the engine into the fixture's own
  cold cache and warm the store template with the installed binary as
  supervisor, snapshot a `TemplateCacheReceipt`, then run the timed first
  launch through a new `kuru_with_stderr` helper (`KURU_OPEN_MARKERS=1`)
  asserting `OPENING` present and `GETTING_READY` absent on stderr; after the
  existing read-only reopen, assert `receipt.verify_used()` and that the
  store's template key equals `template_key()`.
- `packages/kuru-memory/src/store/creation_template/open_tests.rs`: two new
  deterministic tests, `fixture_cache_warm_up_lets_a_fresh_open_copy_with_two_starts`
  and `fixture_cache_receipt_names_what_changed`, pinning the warm-up's start
  count and the receipt's failure-naming behavior.
- `apps/kuru-tui/tests/embedded_runtime.rs`: `packaged_install_and_update_preserve_complete_offline_memory`
  (which stays cold — no warm-up) prints one measurement line with the wall
  time of its cold first launch, visible in the `--nocapture` install job's
  log. Measurement only: no threshold, no assertion on the value.
- `docs/development.md`: one paragraph near the existing store-template
  description noting that the native mise fixture warms its own cache after
  its unchanged cold assertion and before its first launch, and that
  `embedded_runtime` stays cold and prints its first-launch wall time.

## Impact

Test-only. Touches `packages/kuru-memory` (test-support additions, two new
unit tests), `packages/kuru-delivery` (test-support harness only), and
`apps/kuru-tui` (test target: new stderr assertions, new measurement
`eprintln!`). No product, spec, CLI, config or workflow surface changes. No
change to `startup_timeout_secs` or any deadline. Expected to eliminate the
native-mise-fixture member of this flake family on Windows coverage
partitions (the installed binary's own first-launch path on a cold user
machine is unchanged and out of scope). The measurement line this change adds
to `embedded_runtime` is a cheap signal for the `open-time-report` change
(not yet archived), whose phase 2 is expected to turn open-time reporting
into a gate for exactly that cold first-launch path.
