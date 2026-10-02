# Tasks

## 1. `apps/kuru-tui/tests/embedded_runtime.rs`

- [ ] 1.1 Add a pure `fn launch_mode(profile_file: Option<&OsStr>) -> LaunchMode`
      (`LaunchMode::{Cold, Warm}`) that returns `Warm` iff `profile_file` is
      `Some`, and a unit test pinning both arms; verify with
      `cargo test -p kuru --test embedded_runtime launch_mode` (or the
      equivalent `mise run //apps/kuru-tui:test` scoped run) passing locally.
- [ ] 1.2 In `Installation::conversation`, call `launch_mode` once with
      `std::env::var_os("LLVM_PROFILE_FILE").as_deref()`, keep the existing
      cold `ensure!(!self.cache.exists() && !self.data.exists(), ...)`
      byte-identical and first, and feed the same mode value into both the
      warm-up gate and the printed first-launch line; verify by reading the
      diff: the `ensure!` text and position are untouched.
- [ ] 1.3 In `Warm` mode only, before `let started = Instant::now();`:
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
- [ ] 1.4 After the timed launch, in `Warm` mode only: `receipt.verify_used()`
      and `kuru_memory::test_support::store_template_key(&self.data, &scope)?
      == Some(kuru_memory::test_support::template_key())`, where `scope`
      comes from `kuru_memory::test_support::managed_store_scopes(&self.data)`
      asserting exactly one scope; verify these assertions fail (locally,
      reverted before commit) if the warm-up is skipped or a stale template
      is substituted.
- [ ] 1.5 Change the printed first-launch line to show `cold` or
      `warm (coverage)` driven by the same `launch_mode` value used to gate
      the warm-up (fold in the existing `, instrumented` suffix rather than
      keeping both); verify by reading one printed line from each of a local
      uninstrumented run and a run with `LLVM_PROFILE_FILE` set.
- [ ] 1.6 Confirm the failure path is unchanged: a warm-up error or an
      assertion failure after it still surfaces through the same `Err` ->
      `root.keep()` -> `panic!` in
      `packaged_install_and_update_preserve_complete_offline_memory`, naming
      the warm-up's own error context first; verify by reading the panic
      message shape, not by forcing a real failure in CI.

## 2. Docs

- [ ] 2.1 `docs/development.md`: add one sentence next to the paragraph
      documenting #168's native-mise-fixture warm-up (and the paragraph
      stating `embedded_runtime` "stays cold") saying the packaged fixture
      warms the same way under coverage only and stays cold, unconditionally,
      in the Installation job on every OS; verify the existing "stays cold"
      and printed-line-format sentences remain true as written (still
      describing the uninstrumented/install-job case) and `mise run
      docs:check` passes.

## 3. Local verification

- [ ] 3.1 Run `//apps/kuru-tui:test` locally (uninstrumented): the fixture
      stays cold, prints the `cold` line, and all existing assertions pass
      unchanged; record the result as observed evidence.
- [ ] 3.2 Run the same test locally with `LLVM_PROFILE_FILE` set to a scratch
      file path: the fixture warms, prints the `warm (coverage)` line, and
      the new post-launch assertions (`verify_used`, template key) pass;
      record the result, and note that a real coverage run itself prints
      nothing under `--nocapture` rejection, so these local assertions are
      the only direct evidence available before CI.
- [ ] 3.3 Run `mise run //apps/kuru-tui:lint`, `format:check`, `typecheck`,
      and `docs:check`; record pass/fail for each.
- [ ] 3.4 Name explicitly as unrun until this PR's own CI: windows-latest
      coverage partitions green with no rerun, and the Installation job on
      every OS still printing `cold` — both observed from the PR's CI run,
      not manufactured locally.
