# Tasks

Scope note, applying to every group below: nothing here adds a product
caller. `Creation::Default` and `Creation::Cold` are unchanged, so no task
asserts a changed product start count — that is P4b-ii. Where a test named in
the roadmap design's section 9 needs a copy worker or a creation selector
that does not exist yet, the task below lands its library-level equivalent
(calling `ensure_in`/`create_in`/the quarantine or copy functions directly, as
PR #145's tests called adoption and recovery directly against hand-built
stages) and names the open-path half that stays for P4b-ii explicitly, so
nothing is silently skipped.

## 1. Key computation

- [ ] 1.1 Refactor the creation-time SQL and literals the key must cover
      (bootstrap statements and placeholder literals from `server.rs`, with
      the secret excluded; `initialize`'s statements and commit message;
      `AUTHOR`; the attempt and publish commit-message formats; the
      `behavior:` block of `server_yaml`) into named `const` arrays in the
      new template module, each with a doc comment naming the line it was
      extracted from, and verify by `cargo build` with no behavior change
      (the arrays are assembled into the same strings the call sites used
      inline) plus the existing engine-contract and adoption test suites
      still passing unchanged.
- [ ] 1.2 Add a domain tag and `TEMPLATE_FORMAT` constant, and a
      `compiled_template_key() -> &'static str` using a `OnceLock`, hashing
      (length-framed, the `compose` style of `test_support/template.rs`) the
      domain tag, format, `DOLT_VERSION`, the pinned executable digest and
      target triple, `CURRENT_VERSION`, `USAGE_CURRENT_VERSION`, the digest
      of every `REGISTRY`/`USAGE_REGISTRY` definition in order, and the 1.1
      arrays. Verify by unit test `template_key_tracks_schema_engine_statements_and_format_only`
      (T4): changing any one input (a definition's SQL, `CURRENT_VERSION`, a
      literal) changes the key in a focused unit harness that constructs the
      hash from substituted inputs directly (not by editing the real
      registries), while the supervisor-binary path and an unrelated source
      file's text do not.
- [ ] 1.3 Add a compile-time or test-only check that enumerates every
      creation-path constant the key must cover against 1.1's arrays (a list
      comparison, not a derive), and verify it fails when a new keyed input
      is added to a call site without being added to the array it belongs
      to; record as evidence that the check was exercised by temporarily
      adding an unkeyed constant and observing the failure, then reverting.

## 2. Layout, locks and the private template root

- [ ] 2.1 Add `templates/` under `<cache>/<DOLT_VERSION>/` via the existing
      `files::ensure_private_directory` pattern, and `<key>.lock`,
      `<key>/manifest.json` + `<key>/data/`, `.build-<key>-<uuid>/`,
      `.stage-<key>-<uuid>/`, `.rejected-<key>-<uuid>/` path helpers, plus a
      Windows-only `lifecycles/` root passed to the build store's lifecycle
      lock. Verify by a unit test asserting each path helper's shape and that
      `templates/` is created with the same private-directory discipline
      `provision.rs` already uses elsewhere (owner-only, no group/world
      access on Unix; a restrictive Windows DACL).
- [ ] 2.2 Add `ensure_in(root: &Path, …) -> Result<Template>` and
      `create_in(root: &Path, …) -> Result<Template>` as the module's public
      entry points, taking the template root as an explicit parameter (never
      derived from `config.cache_dir` inside the module), matching the shape
      of `test_support/template.rs`'s `instantiate_in(root, …)`. Verify by a
      unit test calling `ensure_in` against an empty private root and
      observing it builds, then calling it again and observing it does not
      rebuild (library-level equivalent of T3 `second_project_reuses_template_without_build`:
      no new `.build-*` directory is created on the second call).
- [ ] 2.3 Add a crate-private `OpenOptions` field (under `cfg(any(test,
      feature = "test-support"))`) carrying an optional private template
      root, read only by test-support code, so T2, T3, T5-T8, T13, T14, T17
      and T20-T22-style tests can exercise the module without disturbing the
      shared job-wide test cache's warmed template. Product code always
      passes `<cache>/<DOLT_VERSION>/templates`. Verify by a unit test that
      the field is absent from `OpenOptions::new`'s public construction path
      (cannot be set outside the crate).
- [ ] 2.4 Library-level concurrency test `concurrent_template_lock_acquisition_never_waits`
      (equivalent to T5's `ensure_in` half): one task holds a key's exclusive
      lock (simulating a build in progress); a second `ensure_in` call for
      the same key under the same private root returns immediately as "no
      template available" rather than blocking, and never reads
      `.build-*`/`.stage-*` directories. Verify by the test observing no
      wait (a bounded test timeout that would fail on any blocking) and the
      ledger counter from 9.x showing zero template builds for the second
      caller.

## 3. Build: the `TemplateBuild` stage-worker job

- [ ] 3.1 Add a `Start::TemplateBuild` (or equivalent) job to
      `store/stage_worker.rs`, run under the key's exclusive lock (held as
      the build server's reap guard, matching the pattern the module's doc
      comment already describes for `Start::Adopt`): bootstrap with
      `TEMPLATE_INSTANCE`/`TEMPLATE_SCOPE`, `initialize`, the full migration
      chain, `upgrade_usage` and `validate_usage` on the usage branch,
      `validate_active`, then the shared shape assertion
      (`store/migrations/template_shape.rs::check`) with `Row::Placeholder`
      and expected counts derived from the compiled registries, on one
      engine. Verify by test `template_shape_counts_derive_from_registries`
      (T27): a synthetic registry set including a usage-only step produces
      the expected branch set and both commit counts from the formula, with
      no hardcoded count in the assertion.
- [ ] 3.2 Add the non-waiting sweep of abandoned `.build-<key>-*` and
      `.stage-<key>-*` entries for the key being built, attempted once before
      a fresh build starts: one quiescence attempt at the smallest valid
      wait (never zero), removal on success, and the entry left in place for
      a later holder on a held lifecycle lease or any removal failure.
      Verify by test `abandoned_build_with_live_lease_is_skipped_without_waiting`
      (T26): a `.build-*` directory with a held lifecycle lease is left,
      removed from nothing, and the build proceeds in its own fresh
      directory with the ledger showing no extra wait beyond the one
      quiescence attempt.
- [ ] 3.3 Verify by test `half_built_template_is_swept_after_quiescence_and_rebuilt`
      (T6): a `.build-*` or `.stage-*` directory left by a simulated
      interrupted build (no engine holding its lifecycle lease) is swept
      before the next build for that key, and the next build succeeds and
      publishes.
- [ ] 3.4 Verify by test `cancelled_build_releases_key_lock_only_after_reap`
      (library-level equivalent of T16): cancelling the build task mid-chain
      leaves the exclusive key lock held until the ledger shows the build
      server reaped, matching `cancelled_open_during_stage_build_keeps_startup_lock_until_reap`'s
      pattern for the existing stage worker.
- [ ] 3.5 Verify by test `non_table_objects_fail_the_template_shape` (T29): a
      view, trigger, procedure, `dolt_schemas`/`dolt_procedures` row or
      `dolt_ignore` rule created on the build engine before the shape check
      runs refuses publication; record which of those objects
      `information_schema`/the Dolt system tables actually show as present
      vs. absent on the pinned 2.3.5 (settling the [unknown] left by spike
      item S11, not present in `engine-contract-findings.md`) as acceptance
      evidence, not assumed from the roadmap design.
- [ ] 3.6 Verify by test `template_born_store_matches_cold_store` (T10,
      parity): build a template, then separately build a cold store on the
      same schema and compare schema, receipts (minus operation UUIDs),
      branch and record version sets, usage version and empty data tables;
      use the comparison to fix `template_shape.rs::BASE_COMMITS` as a
      measured constant (replacing its current "measured on cold store,
      [to be confirmed]"-style doc comment) rather than leaving it a guess.

## 4. Capture, byte scan, manifest and publication

- [ ] 4.1 Promote the capture walk and classification of
      `test_support/template.rs` (roughly lines 430-473 at this base) to the
      production module: skip runtime locks; refuse capture of
      `endpoint.json`, `migration.json`, `sql-server.info`, `*.pid`,
      `*.sock`; `sync_all` every captured file and directory. Verify by test
      `copied_files_and_directories_are_synced_before_identity` (T11): an
      ordered hook shows every file and directory synced before the manifest
      is written.
- [ ] 4.2 Add the byte scan over the captured tree, refusing publication if
      any file contains the build store's absolute path, either build
      secret, or the local host name (covering `.dolt/stats`, per
      `engine-contract-findings.md` S7's inventory). Verify by test
      `template_holds_data_only_without_build_path_secret_or_host_name`
      (T12), and record in its evidence, and in the docs task in group 13,
      that compression can hide repeated text so this scan is a
      defense-in-depth check and the SQL-level shape assertions (group 3)
      are the assertion the design actually relies on (S7's own caveat).
- [ ] 4.3 Write `manifest.json` (format, key, entry list of path/bytes/SHA-256)
      and publish by `files::move_directory(stage, <key>)` (no-replace
      rename, re-opening and checking the destination's identity), with a
      bounded retry in the style of the engine-activation retry
      (`provision.rs`, 2 s at 20 ms). Verify by test
      `publication_failure_copies_from_verified_stage` (T13): an injected
      rename failure leaves the verified stage directory intact and
      reachable by the build's own caller, and leaves it for the next
      exclusive-lock holder to sweep (group 3.2) rather than erroring the
      build.

## 5. Verification on use: structural check and `copy_into`

- [ ] 5.1 Add a structural check (manifest parses, format and key match,
      top-level entries are exactly `{manifest.json, data}`) runnable before
      any stage exists, under the shared lock. Verify by the structural half
      of test `template_failing_structure_is_quarantined_and_open_goes_cold`
      (T7): a wrong-format or wrong-key manifest is a verdict (quarantined,
      group 6); a manifest read error (simulated sharing violation/`EIO`) is
      not a verdict and leaves the template untouched. The "open goes cold"
      half of T7 is explicitly deferred to P4b-ii, which has a cold fallback
      to go to.
- [ ] 5.2 Add `copy_into(stage, published, …)`: per-file size and SHA-256
      verified while copying, entry-set equality against the manifest, and
      refusal of a link, an extra hard link, or an entry of a different type
      than the manifest declares. Verify by the copy half of test
      `template_byte_corruption_mid_copy_preserves_remnant_and_quarantines`
      (T8): a digest or size mismatch is a verdict (quarantined, group 6);
      an injected read or write I/O error on either side is not a verdict,
      preserves the partial copy as a remnant, and triggers no quarantine.
      The "this open continues on the cold path" half of T8 is deferred to
      P4b-ii.

## 6. Quarantine

- [ ] 6.1 Add `CreationFailure::{TemplateVerdict, Engine, Io}` (or the
      equivalent discriminated failure type), in the style of the existing
      `DoltPrematureExit` downcast `Server` uses to decide a restart, so
      every failure source in groups 3-5 reports its discriminant rather
      than a single error type.
- [ ] 6.2 Add the identity-bound quarantine action: under the key's
      exclusive lock, open the published `<key>/` as a checked `Directory`,
      compare its identity to the one recorded when the verdict was reached,
      and rename to `.rejected-<key>-<uuid>` only on a match, dropping the
      comparison handle before the rename (so a held handle cannot block it
      on Windows); best-effort on `WouldBlock`, any lock/identity-comparison
      error, or a failed rename, each logged as a warning and never
      surfaced as an open error; an older `.rejected-<key>-*` removed first,
      also best-effort. Verify by test `quarantine_is_bound_to_the_judged_template`
      (T21): a hook between the verdict and the exclusive-lock acquisition
      replaces `<key>/` with a fresh, differently-identified valid template;
      the quarantine skips, and the fresh template is left published and
      unquarantined.
- [ ] 6.3 Verify by test `template_lock_errors_send_the_opener_cold`'s
      library half (T22): an injected `lock_file` open error, a
      `TryLockError::Error`, and an identity-verification failure each make
      `ensure_in` return "no usable template for this key" rather than an
      error, and quarantine nothing. The "opener completes on the cold path
      with its start count" half of T22 is deferred to P4b-ii, which has a
      cold path to observe it on.
- [ ] 6.4 Verify, as a focused unit test rather than a full open-path
      scenario (the open-path version is P4b-ii's T20), that every
      non-verdict failure path in groups 3-5 (engine crash, lost reply,
      deadline, authentication failure, SQL error unrelated to the expected
      comparison, lock contention, I/O error) reports `Engine`/`Io`, never
      `TemplateVerdict`, and leaves a previously published template for that
      key untouched.

## 7. No garbage collection

- [ ] 7.1 Verify by test `open_never_removes_other_keys_or_lock_files` (T14,
      replacing the design's earlier GC-test shape per review finding B2): a
      private template root pre-populated with two other keys' published
      templates, one other key's `.rejected-*` directory, and lock files for
      all three keys; building and publishing a template for a fourth key,
      and separately copying an existing key's template, each leave every
      other entry byte-identical afterward (contents and mtimes where
      feasible to assert), and remove no lock file.
- [ ] 7.2 Confirm (not implement — `provision.rs`'s `.leftovers` sweep
      already does not enumerate `<DOLT_VERSION>/` subdirectories other than
      what it already reads) that nothing under `provision.rs`'s existing
      install/version bookkeeping is made to notice or sweep `templates/`;
      record the confirming grep as evidence.

## 8. Fixture warm-up: the template cell

- [ ] 8.1 Add a second `tokio::sync::OnceCell` to `test_support::warm_runtime_cache`,
      run after the existing engine cell, calling
      `creation_template::ensure_in(<shared test root>, …)`: verify
      structurally if `<key>/` is published; otherwise poll `try_lock`
      (the `acquire` loop pattern of `test_support/template.rs`) for the
      exclusive key lock, bounded by one fresh-open budget constant plus
      `WARM_UP_MARGIN`, then build and publish. The cell caches only
      success. Verify by test `warm_runtime_cache_builds_one_template_across_processes`
      (T17): the child-process pattern of
      `test_support/template.rs`'s existing cross-process test, run against
      the production template cell, shows exactly one build across
      concurrently started processes sharing a cache.
- [ ] 8.2 Verify by test `failed_template_warm_up_is_not_cached` (T25): an
      injected failure in the first call's template-cell init fails only
      that call (named error, not masked), and the next call builds and
      succeeds.
- [ ] 8.3 Confirm lock-file errors inside warm-up (open, `TryLockError::Error`,
      identity-verification failure) are fatal and named, not silently
      treated as "no template" the way group 6.3's product-facing behavior
      is; record the distinguishing code path and its test as evidence
      (warm-up has no fallback to send a caller to, unlike a product opener).

## 9. Fixture warm-up: the per-fixture token and deterministic guard

- [ ] 9.1 Add a crate-private `Fixture::{Unwarmed, Warmed}` field on
      `OpenOptions` (`cfg(any(test, feature = "test-support"))`), defaulting
      to `Unwarmed` in `test_support::open_options`. Add
      `test_support::warmed_open_options(data, scope) -> Result<OpenOptions>`
      (async) and `OpenOptions::warmed(self) -> Result<Self>` that call
      `warm_runtime_cache` (both cells) and set `Fixture::Warmed`.
- [ ] 9.2 In `open_inner`, after the active store is found absent and before
      `recover_staging` or any start, add the deterministic guard: a
      writable open with `Fixture::Unwarmed` whose template root is the
      shared test root fails at once with the named message; a reopen of an
      existing store is never guarded. Verify by test
      `unwarmed_fixture_open_fails_the_guard_before_any_start` (T23):
      options from `open_options` without `.warmed()` fail immediately with
      zero engine starts recorded in the ledger.
- [ ] 9.3 Update `MemoryStore::temporary()`, `temporary_cold()` to call
      `warm_runtime_cache().await?` first (before `template::instantiate`
      and before `temporary_permit()`), and `open_temporary()` to build its
      options through the warmed builder. Verify by test
      `temporary_warms_before_instantiate` (T24): a child process with a
      fresh private `KURU_DOLT_CACHE` (never the runner's own environment)
      shows the first `temporary()` call publishing the production template
      during warm-up, and the test-template `build()`'s own open records 2
      engine starts and 0 in-open template builds.
- [ ] 9.4 Fail the guard on every fixture call site of
      `test_support::open_options` across `kuru-memory`, `kuru-runtime` and
      `kuru-tui` that is not already covered by 9.3, by switching each to
      `warmed_open_options`/`.warmed()`. Verify: running each affected
      package's test suite once with the guard active and once with it
      behind a temporary bypass to confirm the guard would have caught each
      missed site before the switch, recording the list of call sites
      changed as evidence (the guard's own design intent, per 15.2 B3's
      disposition, is that it "fires on the first run of every unwarmed
      call site").
- [ ] 9.5 Add `TempDir::allowing_template_build()` (or equivalent opt-in) and
      `engine_ledger` counters for a template built inside `open_inner` and a
      template quarantined under the shared root; the fixture teardown guard
      fails a fixture whose open did either unless it opted in. Verify by
      test `fixture_open_that_builds_a_template_fails_the_guard` (T18): a
      fixture using a private, never-warmed root and a writable open that
      reaches the build path fails teardown naming the counter, unless
      `allowing_template_build()` was set.

## 10. Fixture warm-up: spawned owners and `cache_dir`

- [ ] 10.1 Make `test_support::cache_dir()` fallible, warming synchronously
      through a new `warm_blocking()` that runs the async warm-up on a
      private thread with its own current-thread runtime, refusing to run
      if already inside a Tokio runtime (`Handle::try_current().is_ok()`)
      and naming `warmed_cache_dir().await` instead in its error. Verify by
      test `warm_blocking_refuses_inside_a_runtime` (T28): called from
      inside a Tokio runtime, it returns the named error and starts no
      thread.
- [ ] 10.2 Add async `warmed_cache_dir().await` for callers already inside a
      Tokio runtime. Update `spawn_logged_owner` and the service spawn
      helpers to warm with the async form before spawning.
- [ ] 10.3 Update kuru-tui's `cache_dir()` callers
      (`apps/kuru-tui/tests/support/memory.rs`, `directories.rs`,
      `trust.rs`) to the fallible form, and determine whether
      `apps/kuru-tui/src/ui/runtime_tests.rs` runs inside a Tokio runtime
      (the design's [unknown]); use `warmed_cache_dir().await` there if so,
      `cache_dir()` otherwise. Verify by that package's existing test suite
      passing unchanged, and record the Tokio-runtime determination as
      evidence (resolving the [unknown] rather than assuming it).

## 11. Spawn-gate audit

- [ ] 11.1 Audit every `spawn_gate::spawning()` call site in `service.rs`,
      `facade.rs` and `operational_gc_tests.rs` that encloses a store or
      service open, and confirm none of them now runs a first, unwarmed
      warm-up under a held gate (groups 8-9 should already prevent this by
      construction: the guard fails instead of warming inside an open, and
      every fixture builds its warmed options before any gate). Record the
      count of `spawning()` sites and warm call sites found in each file as
      evidence, alongside the roadmap design's own prior count (21/15 in
      service.rs, 17/17 in facade.rs, 3/1 in operational_gc_tests.rs at
      `a96a16ae`), noting any drift.
- [ ] 11.2 Add a regression test or assertion (whichever this audit's
      findings make practical — a lint-style grep check in test-support, or
      a targeted unit test per site class) that a future `spawning()` site
      enclosing an open without a prior warm is caught, not just observed
      once; record what was added and why.

## 12. `prefetch`

- [ ] 12.1 Add a template-warm step to `packages/kuru-memory/mise.toml`'s
      `prefetch` task (`main.rs`), after engine provisioning, using the
      prepared supervisor snapshot it already creates. Verify: running
      `mise run //packages/kuru-memory:prefetch` locally publishes a
      template under the shared cache, and the ordinary `test` task (which
      depends on `prefetch`) shows no in-process template build on its first
      fixture.
- [ ] 12.2 Confirm coverage's tasks (which clear
      `KURU_TEST_SUPERVISOR_PREPARED` and do not run `prefetch`) warm the
      template in the instrumented test process through the fixture path of
      groups 8-9, using the instrumented supervisor; record that the
      template's key excludes the supervisor (group 1), so the instrumented
      and ordinary templates for a given schema/engine are interchangeable,
      and that no ordinary supervisor is compiled or snapshotted first.

## 13. Docs

- [ ] 13.1 `docs/development.md`: document the shared test cache now keeping
      every template key built on the machine, with nothing pruning it, and
      that deleting the directory while no test runs reclaims it; document
      obtaining fixture options through `warmed_open_options`/
      `OpenOptions::warmed()` before any spawn gate, and that an unwarmed
      fixture open fails at once; document the production vs. test template
      distinction; document that `warm_runtime_cache` warms both; document
      `Creation::Cold` for cold fixtures; document kuru-tui's
      `warm_blocking`/`warmed_cache_dir`; document the fixture teardown
      guard and its opt-in.
- [ ] 13.2 `docs/memory.md` and `apps/kuru-docs/concepts/memory.md`: add a
      passage placing the template cache beside the engine cache — what it
      holds (`data/` only: schema, receipts, placeholder identity) and does
      not (no credentials, no project data, no `identity.json`/`config/`/`home/`),
      that it is shared by every project on the machine that shares a
      cache, that an explicit project purge does not touch it, that the
      byte scan guards against path/secret/host leakage but is
      defense-in-depth (the SQL-level shape assertions are the real guard,
      per S7), and that no product open uses it yet (this PR only builds
      and verifies it; the next change routes creation through it).

## 14. Spec and gate

- [ ] 14.1 Confirm the spec delta at
      `specs/versioned-memory/spec.md` (authored in this change) targets
      only `ADDED Requirements` against the living
      `openspec/specs/versioned-memory/spec.md`, and run
      `mise run cospec -- validate memory-template-cache --strict`,
      recording its pass with 0 errors/0 warnings as evidence (or naming and
      resolving whatever it reports).
- [ ] 14.2 Run the package-scoped checks this change touches:
      `mise run //packages/kuru-memory:test`,
      `mise run //packages/kuru-memory:lint`,
      `mise run //packages/kuru-memory:typecheck`, the affected
      `kuru-runtime` and `kuru-tui` test tasks, `mise run format:check`,
      and `mise run docs:check`; record each command's pass/fail and, for
      any skipped or deferred check (for example a full coverage run, which
      CI gates separately), name why it was not run here rather than
      claiming a pass that was not observed.
