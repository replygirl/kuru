# Tasks

## 1. Test-support warm-up primitives

- [ ] 1.1 Add `template_key()`, `store_template_key(data, scope)`,
      `engine_warm_up_bound()` and `TemplateCacheReceipt`
      (`snapshot`/`verify_used`/`Display`) to `packages/kuru-memory/src/test_support.rs`
      and verify with `//packages/kuru-memory:test`.
- [ ] 1.2 Add `fixture_cache_warm_up_lets_a_fresh_open_copy_with_two_starts`
      and `fixture_cache_receipt_names_what_changed` to
      `packages/kuru-memory/src/store/creation_template/open_tests.rs`,
      holding `spawn_gate::spawning()` across `provision`,
      `warm_template_cache` and the open, and verify both pass locally.

## 2. Harness warm-up and assertions

- [ ] 2.1 In `Installation::conversation`
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
- [ ] 2.5 In `packaged_install_and_update_preserve_complete_offline_memory`
      (`apps/kuru-tui/tests/embedded_runtime.rs`, which stays cold — no
      warm-up), print one clearly labelled line with the wall time from
      spawn to the cold first launch's success, e.g. `embedded_runtime first
      launch (cold cache, release binary): <ms> ms [<label>]`. Measurement
      only: no threshold, no assertion on the value, no change to what the
      test otherwise verifies. Verify it appears in `test:embedded-runtime`
      `--nocapture` output locally.
- [ ] 2.2 Run `windows_mise` on native Windows in the worktree and record the
      pass with the new assertions and the printed warm-up/launch timings.
- [ ] 2.3 Run it twice more from fresh roots and record first-launch wall
      times for comparison against the one known passing sample (48.40 s
      total test time, job 110506119241 — not an isolated launch-time
      baseline, the only pre-change passing sample available); expect a
      warm, 2-start open well inside the 30 s wait.
- [ ] 2.4 Throwaway, uncommitted: pass a bogus supervisor path into the
      warm-up and confirm the warm-up error appears first in the failure
      context, with the fixture root retained (R4) — do not commit this run.

## 3. Documentation

- [ ] 3.1 Add one paragraph to `docs/development.md` near the existing
      store-template description (the `420`-ish region) noting the native
      mise fixture warms its own cache (engine, then template with the
      installed binary as supervisor) after its unchanged cold assertion and
      before its first launch, that the first launch then copies with two
      engine starts, that `TemplateCacheReceipt::verify_used` and the
      store's template key prove it, and that the packaged `embedded_runtime`
      fixture stays cold and prints its own first-launch wall time. Verify
      with `mise run docs:check`.

## 4. Local verification

- [ ] 4.1 Run `//packages/kuru-memory:test`, `//apps/kuru-tui:test`, `lint`,
      `lint:windows`, `format:check`, `typecheck`, `docs:check`,
      `cospec:managed:check` locally and record pass/fail for each.
- [ ] 4.2 Confirm CI is green on the branch's own pushed commits — Windows
      coverage partitions in particular — without any rerun (a failure is a
      defect to diagnose, not to rerun past). Note `verify:staged-windows` as
      unrun until the next release.

## 5. Close-out

- [ ] 5.1 `mise run cospec -- validate installed-binary-template-warmup --strict`,
      then archive before merge.
