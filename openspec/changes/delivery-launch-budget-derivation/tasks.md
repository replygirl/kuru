# Tasks

Acceptance evidence is recorded against each task as its check finishes; unrun checks are named with the reason. Affected surfaces: `packages/kuru-delivery` test constants and support (listed in the proposal), the two exposed budgets (`kuru_delivery::update_budget`, `kuru_memory::test_support::fixture_deadline`) and `docs/development.md`.

## 1. Derivation record

- [x] 1.1 Identify, for each of the four 180 s constants, the product or vendor budget it depends on (or the event it should wait for), citing the source budget in code or vendor documentation, and verify the record names a source for every constant and invents no number.
  - Evidence: `tests/support/launch_budget.rs` module docs and the item docs at each constant. Sources: `update_budget::{STARTUP, PUBLICATION, CLEANUP}` (same values at v0.7.0, v0.8.0 and v0.9.0, read with `git show <tag>:packages/kuru-delivery/src/update.rs`); mise `settings.toml` at tag v2026.9.18 (`http_timeout` 30s, `http_retries` 3, documented backoff, `fetch_remote_versions_timeout` 20s); `kuru_memory::test_support::fixture_deadline(1, 0)` (126 s, asserted by its reviewed test); `coverage::shard_deadline` for the launches with no product or vendor budget. No new margin literal.
- [x] 1.2 Write the derivation record beside the constants and verify each constant cites it.
  - Evidence: `launch_budget.rs` is included by `powershell_diagnostics.rs`, `bootstrap_windows.rs` and `mise_acceptance.rs`; `previous_updater.rs::DEADLINE` documents its derivation and cites the record. `launch_bounds_take_their_recorded_derivations` checks each file's derivation text.

## 2. Constants and call sites

- [x] 2.1 Derive or replace `TIMEOUT` in `tests/bootstrap_windows.rs` and verify its call sites still assert the same outcomes.
  - Replaced at both sites (`run()` and `line()`) by `launch_budget::until_job_deadline()`; assertions and expect messages unchanged. Windows-only (`cfg(all(windows, feature = "tooling"))`): compiled and linted locally by `mise run //packages/kuru-delivery:lint:windows` (exit 0); its behavior is not run on this macOS host and is evidenced by the native Windows coverage run.
- [x] 2.2 Derive or replace `DEADLINE` in `tests/support/mise_acceptance.rs` and `tests/support/previous_updater.rs`, exposing an existing product budget to test support only if needed, and verify with `mise run //packages/kuru-delivery:test`.
  - `previous_updater::DEADLINE = kuru_delivery::update_budget::handoff()` (180 s, the Windows parent's serial helper waits); the bare literal in `previous_release_update.rs` generator now takes the same constant. `mise_acceptance` `DEADLINE = mise_stalled_request() + fixture_deadline(1, 0)` = 125.2 s + 126 s = 251.2 s.
  - `mise run //packages/kuru-delivery:test`: exit 0, every binary `ok` (lib 253 passed including `update_budget::tests::handoff_sums_the_parent_waits_in_series`; `powershell_diagnostics` 10 passed). `previous_published_release_updates_to_this_tree` is `#[ignore]` (needs a release candidate binary and public release access) and was not run; CI's `install` and `verify-staged` jobs run it. `mise_acceptance` compiles only for Windows (`apps/kuru-tui/tests/windows_mise.rs`): `mise run //apps/kuru-tui:lint:windows` exit 0; not run on this host.
  - `kuru-memory`: `cargo test -p kuru-memory --all-features --locked --lib fixture_deadline_tests` 2 passed (`fixture_deadline(1, 0)` = 126 s). The full memory suite was not run: the change only lifts `cfg(test)` from existing items.
- [x] 2.3 Derive or replace `WRAPPER_LAUNCH_BUDGET` in `tests/powershell_diagnostics.rs` and verify no remaining literal lacks a derivation by grepping the package tests for `from_secs(180)`.
  - Replaced by `launch_budget::until_job_deadline()` at both `bounded_output` sites. In the ci.yml `coverage` and native-tests `shard` jobs only the test steps that set `KURU_COVERAGE_JOB_MINUTES` run these binaries. The release workflow's ordinary `tests` job records no job, so there the bound is the local window (35 min from the first bounded launch). A recorded job start in that job is a named follow-on. `grep -rn "from_secs(180)" packages/kuru-delivery/tests` finds nothing (exit 1); the pin test enforces the same scan.
- [x] 2.4 Update the pin test in `tests/powershell_diagnostics.rs` to pin the derivation and the shard deadline bound, and verify it passes and fails when a constant is changed without its derivation.
  - `launch_bounds_take_their_recorded_derivations` passes. Mutation: with `previous_updater::DEADLINE` set back to `Duration::from_secs(180)` it failed with "tests/support/previous_updater.rs no longer takes its bound from ..."; restored afterwards. The former serial-fit check is replaced, since a deadline-relative bound fits by construction: every workflow `KURU_COVERAGE_JOB_MINUTES` places a deadline through `shard_deadline`, `remaining_at` equals it and reports a passed deadline, and mise's cited version equals `WORKFLOW_MISE_VERSION` with no `MISE_HTTP_*` override in the isolation environment.

## 3. Documentation

- [x] 3.1 Change the `docs/development.md` open-time line to say exit is stamped after both pipes close and the command is reaped, and verify against the exit-stamp code from commit 5030b4bc.
  - Checked against `open_time/launch.rs`: Unix `until_drained` stamps after `join!(stderr lines, stdout drain, wait)`; Windows stamps after both pipes reach EOF and `child.wait`.
- [x] 3.2 Verify `mise run docs:check` passes.
  - Exit 0.

## 4. Evidence

- [x] 4.1 Run lint, format check and the delivery package tests, record the observed results (Windows-only tests that cannot run on this host are named as unrun, with CI as their evidence), and verify the hk pre-push hook passes.
  - `mise run format:check` exit 0; `//packages/kuru-delivery:lint` exit 0; `//packages/kuru-memory:lint` exit 0 (first run failed on the never-constructed `FirstProject` variant under the feature-only build; that variant is now test-only); `//packages/kuru-delivery:lint:windows`, `//packages/kuru-memory:lint:windows`, `//apps/kuru-tui:lint:windows` exit 0; `//packages/kuru-delivery:typecheck` and `//packages/kuru-memory:typecheck` exit 0; delivery tests as in 2.2.
  - Not run here: the hk pre-push hook (this stage does not push; the push stage runs it), Windows behavior of `bootstrap_windows.rs`, `windows_mise.rs` and the two `cfg(windows)` wrapper tests (native Windows CI), and the ignored previous-release acceptance (CI `install` and `verify-staged`).
