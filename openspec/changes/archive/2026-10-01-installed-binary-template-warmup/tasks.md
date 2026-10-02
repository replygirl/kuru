# Tasks

## 1. Test-support warm-up primitives

- [x] 1.1 Add `template_key()`, `store_template_key(data, scope)`,
      `engine_warm_up_bound()` and `TemplateCacheReceipt`
      (`snapshot`/`verify_used`/`Display`) to `packages/kuru-memory/src/test_support.rs`
      and verify with `//packages/kuru-memory:test`.
      Evidence (local, macOS aarch64, 2026-10-01): `mise run
      //packages/kuru-memory:test` exit 0; lib target 594 passed, 0 failed,
      6 ignored (763 s); every other target ok. `warm_engine` now calls
      `engine_warm_up_bound()` (same value: lock 180 s + probe 15 s +
      margin 5 s). Deviation from the design's signature: `snapshot` takes
      the warm-up's elapsed `Duration` as a second argument, because
      `Display` reports it and the receipt has no other source for it.
- [x] 1.2 Add `fixture_cache_warm_up_lets_a_fresh_open_copy_with_two_starts`
      and `fixture_cache_receipt_names_what_changed` to
      `packages/kuru-memory/src/store/creation_template/open_tests.rs`,
      holding `spawn_gate::spawning()` across `provision`,
      `warm_template_cache` and the open, and verify both pass locally.
      Evidence (local, macOS): both pass in the full suite above and in a
      filtered run with `spawn_gate::tests::no_spawn_guard_encloses_a_test_cache_warm_up`
      (142 spawn guards scanned, 0 violations); neither new call was added
      to that scan's `WARM` list. T1 opens through `MemoryStore::open` under
      the held guard, not `spawn_gated_open`.

## 2. Harness warm-up and assertions

- [x] 2.1 In `Installation::conversation`
      (`packages/kuru-delivery/tests/support/mise_acceptance.rs`), inside the
      result block after the unchanged cold assertion and cleanup arming,
      provision the engine into the fixture's own cold cache under
      `engine_warm_up_bound()`, warm the store template with the installed
      binary as supervisor, and snapshot a `TemplateCacheReceipt`; never
      create the cache directory directly. Add `kuru_with_stderr` and run the
      timed first launch through it with `KURU_OPEN_MARKERS=1`, asserting
      `OPENING` present and `GETTING_READY` absent on stderr; after the
      existing read-only reopen, assert `receipt.verify_used()` and that
      `store_template_key(&kuru_data, scope)` equals `Some(template_key())`.
      Verify by compiling the harness and reading the new assertions back.
      Evidence (local): the harness is compiled only by the `cfg(windows)`
      `windows_mise` target, so `mise run //apps/kuru-tui:lint:windows`
      (clippy `--target x86_64-pc-windows-msvc --all-targets
      --all-features -D warnings`) is its local compile gate: exit 0.
      `mise exec` forwarding a variable set on its own process to the
      child was checked on macOS (`KURU_OPEN_MARKERS=1` seen by `sh -c`);
      the same on the pinned Windows mise is inferred, not observed, and a
      passing coverage partition captures the printed marker list, so the
      harness comment marks the markers as best-effort diagnostics; no
      assertion depends on them.
- [x] 2.5 In `packaged_install_and_update_preserve_complete_offline_memory`
      (`apps/kuru-tui/tests/embedded_runtime.rs`, which stays cold — no
      warm-up), print one clearly labelled line with the wall time from
      spawn to the cold first launch's success, e.g. `embedded_runtime first
      launch (cold cache, release binary): <ms> ms [<label>]`. Measurement
      only: no threshold, no assertion on the value, no change to what the
      test otherwise verifies. Verify it appears in `test:embedded-runtime`
      `--nocapture` output locally.
      Evidence (local, macOS aarch64, debug `cargo test` build, not
      instrumented, not a release binary): `mise run
      //apps/kuru-tui:test:embedded-runtime` exit 0, 5 passed, printing
      `embedded_runtime first launch (cold cache, cargo test build): 8861 ms
      [direct]` and `... 8936 ms [updated]`. The label names the binary's
      source (`KURU_EMBEDDED_TEST_BINARY executable` or `cargo test build`,
      plus `, instrumented` under `LLVM_PROFILE_FILE`) instead of asserting
      "release", since the selected executable is not always a release
      build.
- [ ] 2.2 (deferred at archive: no native Windows host locally) Run `windows_mise` on native Windows in the worktree and record the
      pass with the new assertions and the printed warm-up/launch timings.
      Unrun: no native Windows host is available locally (macOS); verified
      instead by the Windows coverage partitions in CI (task 4.2) and by
      reading the code. Observed in CI (run 36956677170, d37823ed): the test
      passed with the new assertions on windows-latest (job 110681317355);
      the warm-up and launch timings were not printed, because a passing
      coverage partition captures test output, so the marker list it prints
      is also unobserved there.
- [ ] 2.3 (deferred at archive: no native Windows host locally) Run it twice more from fresh roots and record first-launch wall
      times for comparison against the one known passing sample (48.40 s
      total test time, job 110506119241 — not an isolated launch-time
      baseline, the only pre-change passing sample available); expect a
      warm, 2-start open well inside the 30 s wait.
      Unrun: as 2.2. Coverage rejects `--nocapture`, so a passing partition
      prints nothing; the first release's `verify:staged-windows` run prints
      one sample.
- [ ] 2.4 (deferred at archive: no native Windows host locally) Throwaway, uncommitted: pass a bogus supervisor path into the
      warm-up and confirm the warm-up error appears first in the failure
      context, with the fixture root retained (R4) — do not commit this run.
      Unrun (no native Windows host). Replaced by reading the error path: a
      bogus supervisor makes `ensure_in`'s build start fail as
      `CreationFailure::Engine` (never a verdict, so no quarantine), and
      `warm_template_cache` returns `prefetch the store template <root>:
      <engine error>`. The result block adds `warm the installed binary's
      engine cache and store template before its first launch`, then
      `retain_after_error` runs `retire_idle_service` →
      `acquire_maintenance_permit_traced` → `ServiceLock::location`, whose
      `Directory::ensure_private(<data>/memory/locks)` creates each missing
      component (`open_inner` with `create`), so both locks are taken
      uncontended and the retirement succeeds. Expected text (anyhow
      outermost first): `native mise conversation failed; authenticated
      memory retirement completed; fixture root retained in place at
      "<root>"`, then the causes `warm the installed binary's engine cache
      and store template before its first launch` and `prefetch the store
      template ...`. The warm-up error is the first cause under the
      harness's own context; the retained root gains
      `cold-kuru-data/memory/locks/`. This is inferred from the code, not
      observed.

## 3. Documentation

- [x] 3.1 Add one paragraph to `docs/development.md` near the existing
      store-template description (the `420`-ish region) noting the native
      mise fixture warms its own cache (engine, then template with the
      installed binary as supervisor) after its unchanged cold assertion and
      before its first launch, that the first launch then copies with two
      engine starts, that `TemplateCacheReceipt::verify_used` and the
      store's template key prove it, and that the packaged `embedded_runtime`
      fixture stays cold and prints its own first-launch wall time. Verify
      with `mise run docs:check`.
      Evidence (local): `mise run docs:check` exit 0.

## 4. Local verification

- [x] 4.1 Run `//packages/kuru-memory:test`, `//apps/kuru-tui:test`, `lint`,
      `lint:windows`, `format:check`, `typecheck`, `docs:check`,
      `cospec:managed:check` locally and record pass/fail for each.
      Evidence (local, macOS aarch64, all exit 0):
      `//packages/kuru-memory:test` (lib 594 passed, 6 ignored);
      `//apps/kuru-tui:test` (every target ok, 0 failed; `windows_mise` is
      `cfg(windows)` and does not run here); `lint`; `lint:windows` for
      `//apps/kuru-tui`, `//packages/kuru-delivery` and
      `//packages/kuru-memory`; `format:check`; `typecheck`; `docs:check`;
      `cospec:managed:check`.
- [x] 4.2 Confirm CI is green on the branch's own pushed commits — Windows
      coverage partitions in particular — without any rerun (a failure is a
      defect to diagnose, not to rerun past). Note `verify:staged-windows` as
      unrun until the next release.
      Observed (CI run 36956677170 on d37823ed, the implementation commit,
      first attempt, no rerun): conclusion success, every job success or
      skipped. By test name, read from each job log in turn:
      `native_mise_github_backend_installs_and_activates_real_offline_kuru`
      ok on windows-latest coverage partition 5 (job 110681317355; 56.46 s
      test time, against the one pre-change passing sample of 48.40 s, a
      single sample of each, not a launch-time comparison);
      `fixture_cache_warm_up_lets_a_fresh_open_copy_with_two_starts` ok on
      windows-latest partition 3 (job 110681317287) and
      `fixture_cache_receipt_names_what_changed` ok on partition 4 (job
      110681317272), so the Windows `lifecycles` branch and T2's rename ran
      natively. The `embedded_runtime first launch (cold cache,
      KURU_EMBEDDED_TEST_BINARY executable)` line appears twice in each
      Installation log: ubuntu-latest 4644/4679 ms (job 110681317150),
      macos-latest 4878/4510 ms (job 110681317096), windows-latest
      10467/12128 ms (job 110681317023), windows-11-arm 12173/11371 ms (job
      110681364557), for `[direct]`/`[updated]`. One green run is not a
      flake rate: the failure family was 4 of 37 runs before this change.
      Later commits on the branch change only comments and these artifacts.
      `verify:staged-windows` is unrun until the next release.

## 5. Close-out

- [x] 5.1 `mise run cospec -- validate installed-binary-template-warmup --strict`,
      then archive before merge.
      Observed: strict validation passed (0 errors, 0 warnings) before the
      archive; the archive is in the branch's final commit.
