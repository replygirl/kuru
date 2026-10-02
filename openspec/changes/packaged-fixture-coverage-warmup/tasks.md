# Tasks

## 1. `apps/kuru-tui/tests/embedded_runtime.rs`

- [x] 1.1 Add a pure `fn launch_mode(profile_file: Option<&OsStr>) -> LaunchMode`
      (`LaunchMode::{Cold, Warm}`) that returns `Warm` iff `profile_file` is
      `Some`, and a unit test pinning both arms; verify with
      `cargo test -p kuru --test embedded_runtime launch_mode` (or the
      equivalent `mise run //apps/kuru-tui:test` scoped run) passing locally.
- [x] 1.2 In `Installation::conversation`, call `launch_mode` once with
      `std::env::var_os("LLVM_PROFILE_FILE").as_deref()`, keep the existing
      cold `ensure!(!self.cache.exists() && !self.data.exists(), ...)`
      byte-identical and first, and feed the same mode value into both the
      warm-up gate and the printed first-launch line; verify by reading the
      diff: the `ensure!` text and position are untouched.
- [x] 1.3 In `Warm` mode only, before `let started = Instant::now();`:
      `kuru_memory::provision::provision(&MemoryConfig { offline: true,
      cache_dir: Some(self.cache.clone()), ..Default::default() },
      &self.cache)`, then
      `kuru_memory::test_support::warm_template_cache(&self.cache, &engine,
      &self.binary)` with the installed packaged binary as supervisor, then
      `TemplateCacheReceipt::snapshot(&self.cache, warm_up_elapsed)`, the
      whole chain under one `.context("warm the packaged fixture's engine
      cache and store template before its first launch (coverage)")`; never
      create `self.cache` directly. Verify by running the test locally with
      `LLVM_PROFILE_FILE` set to a scratch path and confirming the warm-up
      runs (temporary `eprintln!` or a debugger, removed before commit) and
      the cache exists only because `provision` created it.
- [x] 1.4 After the timed launch, in `Warm` mode only: `receipt.verify_used()`
      and `kuru_memory::test_support::store_template_key(&self.data, &scope)?
      == Some(kuru_memory::test_support::template_key())`, where `scope`
      comes from `kuru_memory::test_support::managed_store_scopes(&self.data)`
      asserting exactly one scope; verify these assertions fail (locally,
      reverted before commit) if the warm-up is skipped or a stale template
      is substituted.
- [x] 1.5 Change the printed first-launch line to show `cold` or
      `warm (coverage)` driven by the same `launch_mode` value used to gate
      the warm-up (fold in the existing `, instrumented` suffix rather than
      keeping both); verify by reading one printed line from each of a local
      uninstrumented run and a run with `LLVM_PROFILE_FILE` set.
- [x] 1.6 Confirm the failure path is unchanged in mechanism: a warm-up error
      or an assertion failure after it still surfaces through the same
      `Err` -> `root.keep()` -> `panic!` in
      `packaged_install_and_update_preserve_complete_offline_memory`, with
      the warm-up's own `.context(...)` as the innermost (first-attached)
      context. Note the two call sites (lines ~1666, ~1728) each wrap
      `conversation` in their own outer `.context("verify the direct/updated
      installation's cold offline memory")`, so `{error:#}` prints that outer
      text first; reword those two call-site contexts to be mode-aware (drop
      "cold" or branch the wording) so the printed chain never claims "cold"
      during a warm-mode failure. Verify by reading the panic message shape,
      not by forcing a real failure in CI.

## 2. Docs

- [x] 2.1 `docs/development.md`: next to the paragraph documenting #168's
      native-mise-fixture warm-up, and the paragraph stating `embedded_runtime`
      "stays cold" and quoting its printed line
      (`embedded_runtime first launch (cold cache, …): <ms> ms [<label>]`):
      update the quoted line to the new `cold`/`warm (coverage)` wording,
      change "stays cold" to state the packaged fixture stays cold
      unconditionally only in the Installation job on every OS, and add one
      sentence that under coverage it warms its cache the same way the native
      mise fixture does before its first launch. `docs:check` does not catch
      a quoted-string mismatch, so re-read the edited paragraph against
      task 1.5's actual output string before committing; then run `mise run
      docs:check`.

## 3. Local verification

- [x] 3.1 Run `//apps/kuru-tui:test` locally (uninstrumented): the fixture
      stays cold, prints the `cold` line, and all existing assertions pass
      unchanged; record the result as observed evidence.
- [x] 3.2 Run the same test locally with `LLVM_PROFILE_FILE` set to a scratch
      file path: the fixture warms, prints the `warm (coverage)` line, and
      the new post-launch assertions (`verify_used`, template key) pass;
      record the result, and note that a real coverage run itself prints
      nothing under `--nocapture` rejection, so these local assertions are
      the only direct evidence available before CI.
- [x] 3.3 Run `mise run //apps/kuru-tui:lint`, `format:check`, `typecheck`,
      and `docs:check`; record pass/fail for each.
- [ ] 3.4 Name explicitly as unrun until this PR's own CI: windows-latest
      coverage partitions green with no rerun, and the Installation job on
      every OS still printing `cold` — both observed from the PR's CI run,
      not manufactured locally.

## Evidence (observed locally, macOS aarch64, 2026-10-02)

- 1.1: `launch_mode_tests::only_a_coverage_destination_warms_the_first_launch`
  (`Some(path)` and `Some("")` -> `Warm`, `None` -> `Cold`) and
  `launch_mode_tests::the_first_launch_line_and_step_contexts_state_the_mode`
  (exact `cold` and `warm (coverage)` lines; the warm step context never says
  "cold") passed in every run below.
- 1.2 (as implemented): `launch_mode` is called once, in `packaged_roundtrip`
  (where `LLVM_PROFILE_FILE` is already read for the private packaging copy),
  and the value is stored on each `Installation`, so the same value gates the
  warm-up, the printed line and the two call-site contexts. The cold
  `ensure!` text and position are unchanged in the diff.
- 1.3: the warm-up reuses #168's `test_support::engine_warm_up_bound`,
  `warm_template_cache` and `TemplateCacheReceipt::snapshot` with the
  installation's own `memory` config; nothing creates `self.cache` except
  `provision`. Nothing was moved: #168's helpers already live in
  `kuru_memory::test_support`, and the native mise fixture is untouched.
- 1.4 (as implemented): the store's template key is read with the `scope`
  the conversation already computes (`kuru_runtime::project_scope`), the scope
  its launches and read-only reopen used, rather than `managed_store_scopes`.
  Warm mode also asserts the first launch's standard error contains the
  opening sentence and not the getting-ready one, as #168 does. Negative
  probe (temporary, reverted, not committed): deleting the published
  template directory after the receipt, under the instrumented run below,
  failed the test with `verify the direct installation's coverage-warmed
  offline memory: the installed binary did not use its warmed store
  template: the warmed store template cache changed after the warm-up of
  store template eef60246… in …/direct/empty-engine-cache/2.3.5/templates
  (30767 ms): eef60246…: republished, so built`, and the fixture root was
  retained.
- 1.5: printed lines: uninstrumented `embedded_runtime first launch (cold,
  cargo test build): 8732 ms [direct]` and `… (cold, cargo test build):
  17758 ms [updated]`; instrumented `embedded_runtime first launch (warm
  (coverage), cargo test build): 1964 ms [direct]` and `… 2946 ms
  [updated]`.
- 1.6: read from code: the warm-up is the first fallible step after the cold
  `ensure!` inside `conversation`, so its error propagates through the same
  `Err` -> `root.keep()` -> `panic!`; `{error:#}` prints the mode-aware call
  site context (`verify the direct installation's coverage-warmed offline
  memory`) then the warm-up context. The negative probe above showed that
  shape for a post-launch failure. The cold call-site strings are unchanged.
- 2.1: `docs/development.md` paragraph updated; quoted line re-read against
  the 1.5 output (`(cold, …)` / `(warm (coverage), …)`).
- 3.1: `mise run //apps/kuru-tui:test:embedded-runtime`, uninstrumented,
  `LLVM_PROFILE_FILE` unset: exit 0, 7 passed, printed the two `cold` lines
  above and `… persisted chat from an empty offline cache`.
- 3.2: a plain run with `LLVM_PROFILE_FILE` set and an uninstrumented binary
  is not possible: the existing packaging-input probe fails with "prepared
  packaging input did not emit one nonempty coverage profile" before any
  installation. Ran instead with real instrumentation:
  `KURU_MBX=0 mise exec -- cargo-llvm-cov llvm-cov -p kuru --test
  embedded_runtime --all-features --locked --no-report -- --nocapture`
  (pinned cargo-llvm-cov 0.9.1): exit 0, 7 passed, printed the two
  `warm (coverage)` lines above and `… persisted chat from a
  coverage-warmed offline cache`; `verify_used`, the template key and the
  stderr assertions passed for both installations. This is a single-test
  instrumented run on macOS, not the workspace coverage task or a Windows
  partition.
- 3.3: `mise run //apps/kuru-tui:typecheck` exit 0; `mise run
  //apps/kuru-tui:lint` exit 0; `mise run //apps/kuru-tui:lint:windows`
  exit 0; `mise run format:check` exit 0; `mise run docs:check` exit 0.
- 3.4: unrun locally (by definition): windows-latest coverage partitions
  green without a rerun, and the Installation job on every OS printing the
  `cold` line; both to be observed from this PR's CI.
