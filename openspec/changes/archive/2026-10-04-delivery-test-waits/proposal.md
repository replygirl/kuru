# Proposal

## Why

Many kuru-delivery tests decide pass or fail on a guessed number: a flat 30 s,
180 s, 10 s, 15 s or 60 s bound on a child run, a download, lock, publication
or release bound shorter than the product ceiling the operation may legitimately
reach, a 200 ms idle race against a paced HTTP fixture, and finite
`sleep 60` keep-alives. `Fixture::run` and `FixtureGit::run` already end on
process exit plus EOF, so only the numbers are guessed. A slow runner then
fails the test, which is a CI flake and a defect in the test, not the product.
Maintainer principle: a wait ends on the event it waits for, bounded by a
derived budget; derive or delete. #204 built the shape-A launch budget
(`tests/support/launch_budget.rs`); this change applies it to the remaining
copies. Behaviour under test is already specified and does not change.

## What Changes

Source of the fix points: `tmp/roadmap/test-wait-inventory-2026-10.md` section 3
("kuru-delivery tests"), section 4 row 4 and section 7.2 row 4: 22 subset fix
points (21 S, 1 M) across 14 files, 202 call sites. Row ids refer to the
per-pass source tables (d1 for `src`, d2 for `tests`).

- Derivation table (kept with the sweep's shared notes at
  `tmp/roadmap/store-creation-design/derivation-delivery-test-waits.md`, each
  constant's derivation also written at the constant and in the
  `launch_budget.rs` doc):
  one entry per fix point naming the governing product budget or event, the
  allowance, and the arithmetic.
- 30 s and 180 s families on the shape-A launch budget: d2#24
  `bootstrap_install.rs` `TIMEOUT`; d2#41 `bootstrap_windows.rs` `TIMEOUT`;
  d2#47 `release_notes.rs` `TEST_TIMEOUT`; d2#58 `windows_update.rs`
  `TIMEOUT` (from `update_budget` STARTUP and CLEANUP); d2#1 and d2#2
  `advisory.rs`; d2#19 `bundle_prepare.rs` `output`; d2#74
  `support/repository_environment.rs`; d2#8 `support/fixture_git.rs` `BOUND`
  (per-git-call allowance, replacing the copy with the #204 budget).
- Ceiling undercuts in the bundle tests, bounded by the product budget the
  operation can reach (`DOWNLOAD_TIMEOUT`, `RETRY_DELAYS[0]`, ICU download
  budget) or by awaiting the event: d1#63 `bundle.rs` inline test module,
  d1#110 `build_tests.rs`, d1#95, d1#99, d1#101, d1#108 `recovery_tests.rs`;
  d1#109 (600 ms absence window) derived from `DEADLINE_RETRY_DELAYS` with
  the arithmetic written at the constant.
- HTTP idle races over `paced_http.rs`: d1#87 `archive/tests.rs` and d1#77
  `published.rs` inline test module, replaced by a signal that the fixture
  delivered headers, or a derived multiple.
- Keep-alives: d2#26 `bootstrap_install.rs` curl stub, d2#11
  `fixtures/delivery.rs` (4 sleeps), replaced by peers that block until
  released; d2#5 `advisory.rs` 250 ms clock started after the ready marker.
- d2#64 `windows_update.rs` late-header clock (size M): done with an
  injectable clock only if it can be done without touching product code under
  `src`; otherwise it is the one item explicitly deferred, with its reason
  recorded in the evidence. No other item may be deferred.
- Deterministic tests pin each shape (derivation arithmetic, blocks until
  released, event-ordered header delivery) rather than asserting elapsed time.
- A new helper is added only where it replaces three or more sites.

## Non-goals

- No change to product code under `packages/kuru-delivery/src` (PR 3's files
  and all other product code). Edits in `src` are confined to test-only files
  and to `#[cfg(test)]` modules (`bundle.rs`, `published.rs` inline tests),
  plus two `cfg(all(test, feature = "tooling"))` lines in `lib.rs` that give
  the lib's fixture Git include its sibling launch budget.
- No retry, no raised literal, no change to what any test asserts.
- No other crate (one crate per PR).

## Impact

- Test-only: `packages/kuru-delivery/tests/**` and the test modules listed
  above; `cog.toml` maps `test` to a patch bump.
- CI time unchanged in the passing case (waits end on events); a stalled
  launch is reported by its derived budget with diagnostics instead of a
  guessed one.
