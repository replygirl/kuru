# Tasks

No product or test code changes: this change is a diagnosis and a sweep.
Tasks verify the measured facts and confirm the sweep found nothing left
to gate, rather than author a fix.

## 1. Confirm #155's fix was already present at the failing commit

- [x] 1.1 Confirm `70fa6c0a` (#155) is an ancestor of `origin/main`'s tip
      and of the exact commit CI tested for PR #133 — verify by
      `git merge-base --is-ancestor` and by reading `tests.rs` at that exact
      SHA.
      Observed: `git merge-base --is-ancestor 70fa6c0a HEAD` (HEAD =
      `0e562595`) succeeds. The tested commit was `5c689e6383031004e4991a565a0de4f1eef87dbe`
      ("Merge 0910c326... into 0e562595..."), itself a descendant of
      `0e562595`. `git show 5c689e638...:packages/kuru-memory/src/store/creation_template/tests.rs`
      shows `template_failing_structure_is_quarantined` already calling
      `create_unspawned` at both call sites (the manifest-read-error case
      and the loop's damage case), byte-identical to the fixed version on
      `origin/main`.
- [x] 1.2 Fetch the failing job's log and record the exact failure —
      verify by reading it directly, not by re-running CI.
      Observed: `gh api --allow-escape-sequences repos/replygirl/kuru/actions/jobs/110720770956/logs`,
      job "Coverage partition (macos-latest, 1)" on run `36969597416`:
      `store::creation_template::tests::template_failing_structure_is_quarantined`
      FAILED with `Error: another format: not quarantined`; `test result:
      FAILED. 152 passed; 1 failed`. This is the "another format" case of
      the loop (the second case; the first, "another key", passed).

## 2. Trace the chokepoint that let the race recur despite #155

- [x] 2.1 Confirm `quarantine()`/`quarantine_held()` in `creation_template.rs`
      implement the documented best-effort skip (non-waiting exclusive
      `try_key_lock`, `WouldBlock` → `Quarantine::Skipped`) and that
      `create_unspawned`'s `spawn_gate::locking_async()` guard spans the
      whole `create_in` call that reaches it — verify by reading both
      functions.
      Observed: `creation_template.rs::quarantine` (~line 1385) calls
      `try_key_lock(root, key, Mode::Exclusive)`, treating `Ok(None)`
      (`WouldBlock`, per `try_key_lock`'s `Mode` match ~line 394-400) as
      `skipped("the store template key lock is busy")`. `tests.rs::create_unspawned`
      (~line 542) binds `_gate = crate::spawn_gate::locking_async().await`
      for its whole body, including the `create_in` call that reaches
      `quarantine()`. The guard is correctly scoped; it just cannot exclude
      a spawn that never takes `spawn_gate`'s own guard.
- [x] 2.2 Enumerate every process-spawn site in `packages/kuru-memory/src`
      and classify each as gated (`spawn_gate::spawning`/`spawning_blocking`
      held across the actual `.spawn()`) or not — verify by reading each
      call site, not by trusting the module doc's fixture list.
      Observed: every spawn site under `#[cfg(test)]` test modules already
      named in `spawn_gate.rs`'s own doc comment (`server_tests.rs`,
      `store/recovery_tests.rs`, `store/migration_lifecycle_tests.rs`,
      `store/operational_gc_tests.rs`, `provision/native_tests.rs`,
      `store/engine_contract_tests.rs`, `store/template_stage_tests.rs`,
      `store/migrations/main_pool_classification_tests.rs`,
      `store/creation_template/tests.rs`'s own `spawn_child`,
      `test_support/template.rs`) takes `spawn_gate::spawning()` or
      `spawning_blocking()` immediately around its `.spawn()` call, or
      routes through `test_support::spawn_gated_open` (shared guard across
      the whole `MemoryStore::open`). One path does not: production
      `server.rs::open_inner_with_probe_delay` (unix ~line 631-658, windows
      ~line 700-730) takes `command.spawn()` under a caller-supplied
      `_test_spawn_guard: Option<RwLockReadGuard>` parameter — but that
      parameter is `None` on the path reached from `Server::open_inner` /
      `Server::open_with_guard` (server.rs ~line 546-549), which is what
      `store.rs::open_inner` calls (~line 2003, unconditionally for the
      existing-store path `temporary()` takes, and again at ~line 2047 for
      the post-migration reopen). Only `Server::open_with_initial_probe_delay`
      (`#[cfg(test)]`, one startup-probe test) supplies a real
      `spawn_gate::spawning()` guard through that parameter.
- [x] 2.3 Confirm `warm_runtime_cache()` does not cover the per-call
      supervisor spawn — verify by reading it and its `OnceCell` caching.
      Observed: `test_support.rs::warm_engine` (~line 192) takes
      `spawn_gate::spawning()` only inside a `tokio::sync::OnceCell::get_or_init`
      closure that runs exactly once per process (the one-time provisioning
      probe). `warmed_open_options`/`temporary()` call `warm_runtime_cache()`
      before opening, but the open itself (the actual per-call supervisor
      spawn traced in 2.2) runs after the gate from 2.1 is long released,
      ungated.
- [x] 2.4 Confirm `MemoryStore::temporary()`/`temporary_cold()` are the
      call sites that reach the ungated path, and count their use across
      the crate — verify by reading `store.rs::temporary`/`temporary_cold`/
      `open_temporary` and grepping call sites.
      Observed: `store.rs::temporary` (~line 2182) calls
      `Self::open_temporary` (~line 2210) which calls `Self::open_inner`
      (store.rs, ~line 2232) with no guard taken before or after; that
      `open_inner` reaches the unconditional `Server::open_with_guard` at
      ~line 2003. `grep -rn "::temporary()\|::temporary_cold()" src` counts
      92 call sites across `store.rs`, `facade.rs`, `service.rs`, and other
      files in the crate's test suite.
- [x] 2.5 Confirm the coverage job's test concurrency makes the race
      reachable, not merely theoretical — verify by finding the CI
      thread-count configuration.
      Observed: `packages/kuru-memory/mise.toml:56` sets
      `env.RUST_TEST_THREADS = "2"` for the coverage task (confirmed also
      at `mise.toml:157`); the job log's own env block (job
      110720770956) does not override it. Two concurrent test threads is
      enough for one ungated `temporary()`/`temporary_cold()` call on the
      other thread to race the exclusive-lock try inside this test's
      write guard.
- [x] 2.6 Look for supporting (not conclusive) evidence that an ungated
      spawn was active during the failing run — verify by reading the job
      log for a test that both finished close to the failure report and
      calls `temporary()`/`temporary_cold()` directly.
      Observed: the job log shows
      `store::migrations::publication_record_tests::unrecorded_failed_attempt_is_still_fully_classified`
      finishing in the same coverage partition before the "failures:"
      block printed; reading it
      (`publication_record_tests.rs:477-478`) shows it calls
      `MemoryStore::temporary_cold().await?` directly, ungated. This is
      supporting evidence only — cargo prints a binary's failures after
      every test in it completes, so temporal proximity in the log does
      not establish the exact-moment lock overlap; no log evidence
      identifies which concurrent spawn, if any single one did, held the
      flock duplicate at the critical instant. Recorded in proposal.md as
      an explicit inference, not a proven cause.

## 3. Sweep every quarantine assertion in the two scoped files

- [x] 3.1 List every assertion in
      `packages/kuru-memory/src/store/creation_template/tests.rs` that
      checks a quarantine outcome (`published(&root).is_none()`/`.is_some()`
      or `rejected(&root)` immediately after a verdict against an
      already-published template) and record its treatment — verify by
      reading each one in context.
      Observed, by function:
      - `template_failing_structure_is_quarantined` (both the
        manifest-read-error case and all 4 loop cases): gated via
        `create_unspawned` (#155, confirmed present at 1.1).
      - `template_byte_corruption_mid_copy_preserves_remnant_and_quarantines`
        (both the injected-read-error case and all byte-corruption-case
        loop iterations): gated via `create_unspawned` (#155).
      - `quarantine_is_bound_to_the_judged_template`: gated via
        `create_unspawned` (#155).
      - `structural_verdict_without_waiting_quarantines_and_never_rebuilds`'s
        first assertion (non-waiting verdict, line ~678): gated — its own
        `ensure_in(&root, &no_engine(), Wait::Never)` call runs inside an
        explicit `spawn_gate::locking_async()` block (predates #155's
        helper but the same pattern).
      - `structural_verdict_without_waiting_quarantines_and_never_rebuilds`'s
        second assertion (warm-up rebuild, line ~710-718): untreated, by
        design, correctly — this path takes `Wait::Until(deadline)` and
        *waits* for the exclusive lock rather than trying once and
        skipping, so the best-effort `WouldBlock` race this change is
        about does not apply to it; it correctly holds the shared
        `spawn_gate::spawning()` guard (it itself spawns the warm-up
        engine), not the exclusive one.
      - `a_busy_key_lock_skips_the_quarantine_and_keeps_the_older_one`:
        deliberately pins the designed skip (#155); not a "the quarantine
        must happen" assertion, left as-is.
      - `publication_failure_copies_from_verified_stage` (line ~1011) and
        `non_table_objects_fail_the_template_shape` (line ~1508): both
        assert `published(&root).is_none()` after a *refused build*, with
        no pre-existing published template to race over (root starts
        empty; no `clone_shared` call precedes them) — not instances of
        this race, no gating needed.
- [x] 3.2 Same sweep for
      `packages/kuru-memory/src/store/creation_template/open_tests.rs` —
      verify by reading each quarantine assertion in context.
      Observed: `damaged_templates_send_the_opener_cold_and_preserve_copy_remnants`
      (structure and digest cases, ~line 726-735),
      `shape_verdict_on_the_copy_fails_the_open_and_quarantines_the_template`
      (~line 780-824), and `adoption_verdict_quarantines_and_leaves_the_stage_in_place`
      (~line 1224-1268) all assert a quarantine after an *open* that itself
      spawns through `test_support::spawn_gated_open`, which takes only the
      shared `spawning()` guard for the whole open — there is no exclusive
      guard available to extend over the verdict without also excluding the
      open's own spawn from itself, which `spawn_gated_open`'s single
      shared guard already serializes against new writers but cannot
      convert to exclusive mid-open. This is exactly the gap #155's commit
      message recorded as a follow-on, not fixed here, and not newly found:
      left untreated with that reason, matching #155's record.
      `child_process_fixture_open_quarantines_the_shared_template` and
      `fixture_open_that_quarantines_the_shared_template_fails_teardown`
      (~line 1299-1365) assert a quarantine of the *shared, cross-process*
      template cache from inside a spawned child test process, a
      different mechanism (cross-process, not cross-thread-in-this-binary)
      not addressed by `spawn_gate` at all; left untreated, out of scope.
- [x] 3.3 Confirm the sweep in 3.1/3.2 finds no additional call in either
      file that both (a) asserts a quarantine outcome and (b) is reachable
      through an exclusive `spawn_gate::locking_async()` guard without
      already having one — verify by cross-checking 3.1/3.2 against the
      full `grep -n "quarantine\|rejected(&root)\|published(&root)"` match
      list for both files.
      Observed: every match in both files is accounted for in 3.1/3.2;
      none is both ungated and gate-able by this change's own pattern.

## 4. Local verification

- [x] 4.1 Run `mise run //packages/kuru-memory:test -- store::creation_template::`
      locally and record the pass count, confirming the sweep's "no code
      change" conclusion is consistent with a clean local run (this cannot
      reproduce the cross-binary race on demand, per #155's own prior
      finding; it confirms no regression, not the race's absence) —
      verify by reading the test binary's own summary line.
      Observed (local macOS arm64, worktree
      `tmp/worktrees/test-memory-template-quarantine-gate`): lib binary
      `test result: ok. 48 passed; 0 failed; 0 ignored; 0 measured; 550
      filtered out; finished in 125.08s`, including
      `template_failing_structure_is_quarantined`. No regression; this run
      does not and cannot exercise the cross-test race on demand.
- [x] 4.2 Run `mise run format:check` and `//packages/kuru-memory:typecheck`
      and confirm they pass — verify by reading each command's exit code.
      Observed: `mise run format:check` exit 0 (Rust formatting plus the
      docs app's `oxfmt --check`, all files already correctly formatted).
      `mise run //packages/kuru-memory:typecheck` exit 0. No `:lint` run
      separately: this change adds no Rust source, only Markdown under
      `openspec/changes/`; hk's own pre-commit/pre-push hooks (format,
      lint, typecheck, cospec, tooling, conventional-commit) run on the
      commit that records this evidence and are the gate of record for
      that commit.
