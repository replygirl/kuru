# Tasks

Acceptance evidence is recorded against each task as its check finishes; unrun checks are named with the reason. The full derivation table (row id, site, what it bounds, governing budget, derived value or deletion, pinning test) is kept with the sweep's shared notes at `tmp/roadmap/store-creation-design/derivation-delivery-test-waits.md`; each derivation is also written at its constant or site. Affected surfaces: `packages/kuru-delivery/tests/**`, the `#[cfg(test)]` modules `src/bundle.rs` (inline tests), `src/bundle/recovery_tests.rs`, `src/bundle/build_tests.rs`, `src/archive/tests.rs`, `src/archive/paced_http.rs`, `src/published.rs` (inline tests), and two `cfg(all(test, feature = "tooling"))` lines in `src/lib.rs`. No product code changes.

## 1. Derivation table and shared launch budget

- [x] 1.1 Write the derivation table, one row per fix point (22 rows, ids d1#63, d1#77, d1#87, d1#95, d1#99, d1#101, d1#108, d1#109, d1#110, d2#1, d2#2, d2#5, d2#8, d2#11, d2#19, d2#24, d2#26, d2#41, d2#47, d2#58, d2#64, d2#74), naming the governing product budget or event and the arithmetic, and verify every row is either done or deferred with a reason
  - Evidence: the table has all 22 rows, lines re-measured on `cbebf7b7`. 20 are done in this change, d2#41 was already done by #204 (`bootstrap_windows.rs` uses `until_job_deadline()` at :208 and :366 on `cbebf7b7`; no edit here), and d2#64 is the one deferred item (5.1). Two dependent sites cleared by the same derivations are also tabled: `advisory.rs:754` (re-executed hostile-config test, 60 s) and `paced_http.rs` `FIXTURE_BOUND` (deleted, see 4.1).
- [x] 1.2 Extend `tests/support/launch_budget.rs` with the budgets the remaining copies need, each with its derivation at the constant, and verify a pin test states the arithmetic
  - Evidence: the module doc lists every new consumer and cites the private product budgets each launch can reach (advisory Git 60 s and scan 180 s, bundle lock 180 s and download 120 s, Communique 600 s, release Git/`cog` 180 s), all below the job deadline. New constant `CHILD_START_ALLOWANCE` (5 s), stated as kuru-memory's `CHILD_START_MARGIN` is, used only where the bound is the stimulus. `launch_bounds_take_their_recorded_derivations` (`tests/powershell_diagnostics.rs`) gains one entry per consumer file with the expected count of each derivation and a zero count for each removed literal or finite keep-alive. Mutation: with `fixture_git::bound()` set back to `Duration::from_secs(10)` it failed ("tests/support/fixture_git.rs no longer takes its bound from …"); restored, it passes.

## 2. Launch-budget families (tests/)

- [x] 2.1 Replace `TIMEOUT` in `bootstrap_install.rs` and `bootstrap_windows.rs`, `TEST_TIMEOUT` in `release_notes.rs`, and `TIMEOUT` in `windows_update.rs` (d2#24, d2#41, d2#47, d2#58) with the shared budget and verify the files compile and their tests pass
  - d2#24: const deleted; its six waits take `until_job_deadline()` (the stub `curl` ignores the script's `--max-time`, so no product budget runs on that path). The file is now `cfg(all(unix, feature = "tooling"))`, since `launch_budget.rs` needs the tooling feature; every task that runs it passes `--all-features`.
  - d2#41: done by #204, unchanged.
  - d2#47: `fn test_timeout()` = `until_job_deadline()` at its four uses.
  - d2#58: `const TIMEOUT: Duration = launch_budget::update_handoff();` (190 s, the updating parent's serial waits including the 120 s publication). Windows-only (`cfg(all(windows, feature = "tooling"))`): compiled and linted locally by `mise run //packages/kuru-delivery:lint:windows` (exit 0); not run on this macOS host. Native evidence is pending CI.
  - `bootstrap_install` and `release_notes` pass in the package test runs in 6.1.
- [x] 2.2 Replace the literals in `advisory.rs`, `bundle_prepare.rs`, `support/repository_environment.rs` and `support/fixture_git.rs` `BOUND` (d2#1, d2#2, d2#19, d2#74, d2#8) and verify the same
  - `until_job_deadline()` at `scan_cli`, the five 30 s sites, `bundle_prepare::output` and the re-executed foreign-repository child; `release_workflow.rs` gains the `launch_budget` include for it. `fixture_git::BOUND` becomes `fn bound()` delegating to the includer's sibling `launch_budget` module (advisory.rs, the fixture binary, and the lib's test-only include in `src/lib.rs` with `extern crate self as kuru_delivery`). Tests pass in 6.1.
- [x] 2.3 Start the `advisory.rs` 250 ms clocks after the observed ready marker (d2#5) and verify the timeout path still fires and is asserted
  - `bounded_output` sets its deadline at spawn, and its timeout is the stimulus these two tests require, so no observed marker can start that clock without a product edit. Taken instead as shape A's stated child-start allowance (`CHILD_START_ALLOWANCE`, 5 s); every run waits it out, so a longer allowance costs time, never a result. Both tests still assert "tool timed out" and the cleanup text. They passed in every run in 6.1 (`advisory` binary 10/10 in a loop).

## 3. Ceiling undercuts in the bundle tests (src test modules)

- [x] 3.1 Derive the `recovery_tests.rs`, `build_tests.rs` and `bundle.rs` test-module bounds from DOWNLOAD_TIMEOUT, RETRY_DELAYS[0] and the ICU download budget, or await the event (d1#63, d1#95, d1#99, d1#101, d1#108, d1#110) and verify no literal shorter than the product ceiling remains
  - d1#63: the poll now ends on the staged partial chunk or on the download task ending (panicking with its result), with no bound of its own. The same test's client total becomes `DOWNLOAD_TIMEOUT`, and its server awaits the `released` oneshot with no timer.
  - d1#95: `client()` total = `DOWNLOAD_TIMEOUT` (the production client's).
  - d1#99, d1#101, d1#108: `PRODUCTION_PREPARATION = LOCK_TIMEOUT + DOWNLOAD_TIMEOUT`, since every wait a preparation awaits is product-bounded. For d1#101 the comment states the causal retry check: the request count plus `unwrap_err`.
  - d1#110: client total = `ICU_DOWNLOAD`.
  - The remaining 8 s/4 s/3 s guards in `recovery_tests.rs` wrap test-chosen budgets and are not subset rows (listed in the table's "not in the subset" note).
  - Lib `bundle::` tests: 10/10 in a loop.
- [x] 3.2 Derive the 600 ms absence window from DEADLINE_RETRY_DELAYS[0] with a written multiple (d1#109) and verify the arithmetic is pinned
  - `DETACHED_RETRY_WINDOW = DEADLINE_RETRY_DELAYS[0] + DEADLINE_RETRY_DELAYS[1]` (1.25 s): the backoff after which a detached retry would reconnect, plus the policy's second backoff as the reconnect allowance. This is written at the constant as the policy's own sum rather than a chosen multiple. It cannot be causal, which the comment states. Pinned by `derived_bounds_follow_the_budgets_they_are_written_from`, which also pins `PRODUCTION_PREPARATION`.

## 4. paced_http idle races and keep-alives

- [x] 4.1 Replace the 200 ms idle races in `archive/tests.rs` and the `published.rs` test module with header-delivery signalling or a derived multiple (d1#87, d1#77) and verify the tests assert the same outcomes
  - reqwest 0.13.5 starts the send phase's read-timeout sleep when the request is created and polls it before the in-flight future (`async_impl/client.rs:2678`, `:3105`). No server-side signal can order header delivery before it.
  - d1#77 (`published`): `fetch` composes `send` then `bounded_body`, so `send` returning is the headers-received event. The test runs on a paused Tokio clock, holding a `spawn_blocking` task until `send` returns. A running blocking task inhibits auto-advance (tokio 1.53.1 `runtime/blocking/schedule.rs:25`), so the 200 ms idle can elapse only in the body phase. Results: 300/300 sequential and 800/800 across 8 parallel loops.
  - d1#87 (`archive`): `download` is monolithic, so the test has no send-phase witness. It takes the production bounds (`CONNECT_TIMEOUT`, `READ_IDLE_TIMEOUT`) under an outer guard of `CONNECT_TIMEOUT + 2 × READ_IDLE_TIMEOUT`, so the test costs 30 s (observed 30.0 to 30.2 s, 3 runs).
  - `paced_http.rs` `FIXTURE_BOUND` (15 s) is deleted: the server is owner-bounded, and a shorter fixture timer would close a 30 s stalled body first, passing the phase assertion without the client's own timeout.
  - Assertions are unchanged in both tests.
- [x] 4.2 Replace the finite `sleep 60` keep-alives in `bootstrap_install.rs` and `fixtures/delivery.rs` with peers that block until released (d2#26, d2#11) and verify the group cleanup tests still pass and nothing outlives them
  - d2#26: the stub producer opens `/dev/tcp` to the fixture's never-accepting loopback listener before its ready marker, then blocks in `cat <&3` until it is killed or the listener is dropped.
  - d2#11: the four fixture modes read stdin to EOF (`hold_until_released`). The stdin is a pipe whose write end the test holds (`held_until_released`, advisory.rs), and every descendant inherits it.
  - Pins: `a_stalled_producer_ends_when_the_fixture_releases_it` waits for the producer's own exit after release under `until_job_deadline` and asserts it is no longer listed. `bounded_fixture_keep_alive_ends_on_its_release` shows a released peer exits 0. The existing cleanup tests still assert no survivor and pass (`bootstrap_install` 5/5 and `advisory` 10/10 in loops).

## 5. windows_update late-header clock

- [x] 5.1 Make the late-header test independent of a 2 s sleep against a 3 s budget through an injectable clock (d2#64), or record it as the one deferred item with its reason, and verify which
  - Deferred, the one permitted deferral. `receive_publication` (src/update.rs:682) uses Tokio timers, so a paused clock reaches them without a product edit. The frame deadline, however, must elapse only after the product has received the late first byte: `frame_deadline` is computed inside `receive_publication` on receipt. That receipt is not observable outside the product function; the fixture can witness only its own write.
  - Ordering a clock advance after the receipt would depend on Windows pipe/IOCP completion ordering, which this host cannot run (the test is `cfg(windows)`). Every real IO wait in that test would also need auto-advance inhibition.
  - A reliable fix needs an observable first-byte event in `receive_publication` (a product edit, a non-goal) or native Windows verification. The test is unchanged apart from its `TIMEOUT` (d2#58).

## 6. Verification

- [x] 6.1 Run the kuru-delivery test, lint (host and Windows target), format and typecheck tasks and the cospec checks, and record observed results and unrun checks with reasons
  - All results are local, on macOS arm64.
  - `mise run //packages/kuru-delivery:typecheck`: exit 0. `//packages/kuru-delivery:lint`: exit 0. `//packages/kuru-delivery:lint:windows`: first run failed (the `Duration` import in `advisory.rs` was unused on Windows, where its users are `cfg(unix)`); after gating the import on `cfg(unix)`, exit 0, and exit 0 again on the committed tree at c7eec6de.
  - `mise run format:check`: first run failed on rustfmt layout after later edits; after `format:rust:fix`, exit 0.
  - `mise run //packages/kuru-delivery:test`: exit 0 in two full runs (the second after the final formatting), every binary `ok` (lib 253 passed, advisory 17, bootstrap_install 28, powershell_diagnostics 10; `previous_published_release_updates_to_this_tree` is `#[ignore]` and was not run, as before).
  - Repetition on the built test binaries: advisory 10/10, bootstrap_install 5/5, bundle_prepare 10/10, lib `bundle::` 10/10, powershell_diagnostics 5/5, release_notes 3/3 and release_workflow 2/2. The last two ran under the task's pinned `cocogitto`/`communique` via `mise exec`; run outside that environment, release_notes fails on the unpinned Communiqué, which is unrelated to this change.
  - `mise run cospec -- validate delivery-test-waits --strict`: 0 errors, 0 warnings. `mise run cospec:managed:check`: no drift.
  - Not run here: the Windows-only `windows_update.rs` and `bootstrap_windows.rs` (verified by `lint:windows` only; native evidence is CI's) and coverage (CI-enforced).
- [ ] 6.2 Run `mise run cospec -- validate delivery-test-waits --strict`, open the PR, and record CI evidence from job logs for the final head
  - Not done in this stage: the PR is opened and CI evidence recorded by the orchestrating session after push.
