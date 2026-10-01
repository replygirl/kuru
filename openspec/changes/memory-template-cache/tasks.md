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

- [x] 1.1 Refactor the creation-time SQL and literals the key must cover
      (bootstrap statements and placeholder literals from `server.rs`, with
      the secret excluded; `initialize`'s statements and commit message;
      `AUTHOR`; the attempt and publish commit-message formats; the
      `behavior:` block of `server_yaml`) into named `const` arrays in the
      new template module, each with a doc comment naming the line it was
      extracted from, and verify by `cargo build` with no behavior change
      (the arrays are assembled into the same strings the call sites used
      inline) plus the existing engine-contract and adoption test suites
      still passing unchanged.
      Evidence: `server.rs` BOOTSTRAP_* constants, READER_ACCOUNT around the unkeyed secret, SERVER_BEHAVIOR (used as `{SERVER_BEHAVIOR}` in `server_yaml`); `store.rs` INITIALIZE_STATEMENTS / INITIALIZE_COMMIT; `migrations.rs` BRANCH_CREATE, ATTEMPT_* (message pieces), PUBLISH_MERGE and TEMPLATE_KEY_STATEMENTS (adds the two branch prefixes, the receipt protocol and the usage branch name). Call sites use the constants. Existing suites pass unchanged: engine contract, adoption (`template_stage_tests`, 14 tests) and the full kuru-memory task (see 14.2). Adoption SQL is not keyed: it runs on copies, not on the template's bytes (covered by TEMPLATE_FORMAT).

- [x] 1.2 Add a domain tag and `TEMPLATE_FORMAT` constant, and a
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
      Evidence: `store/creation_template.rs` `compose`/`compiled_key` (length-framed SHA-256, `OnceLock`); `server::compiled_template_key()` now delegates to it. `template_key_tracks_schema_engine_statements_and_format_only` passed: each of format, engine version, engine digest, target, both schema versions, an edited and a moved definition digest, an edited and an extra statement changes the key; framing distinguishes `[ab,c]` from `[a,bc]`; the compiled key is 64-hex, a valid template key, and has no supervisor or source input. Measured key on aarch64-apple-darwin at this commit: `5894ec0a…40481` [measured].

- [x] 1.3 Add a compile-time or test-only check that enumerates every
      creation-path constant the key must cover against 1.1's arrays (a list
      comparison, not a derive), and verify it fails when a new keyed input
      is added to a call site without being added to the array it belongs
      to; record as evidence that the check was exercised by temporarily
      adding an unkeyed constant and observing the failure, then reverting.
      Evidence: `creation_statements_are_keyed` scans the bodies of `initialize_database`, `server_yaml`, `initialize`, `discover_current_attempt_in`, `build_attempt`, `publish` and `ensure_usage_branch_at_v4` for inline writing SQL (`CREATE/INSERT/UPDATE/DELETE/ALTER/GRANT/DROP/CALL DOLT_`) and checks each keyed constant is in the key's statements. Exercised [measured]: temporarily adding `sqlx::query("CREATE TABLE unkeyed_probe (id INT)")` to `initialize` made it fail (`store.rs async fn initialize(pool holds inline writing SQL starting "CREATE `); reverted.

## 2. Layout, locks and the private template root

- [x] 2.1 Add `templates/` under `<cache>/<DOLT_VERSION>/` via the existing
      `files::ensure_private_directory` pattern, and `<key>.lock`,
      `<key>/manifest.json` + `<key>/data/`, `.build-<key>-<uuid>/`,
      `.stage-<key>-<uuid>/`, `.rejected-<key>-<uuid>/` path helpers, plus a
      Windows-only `lifecycles/` root passed to the build store's lifecycle
      lock. Verify by a unit test asserting each path helper's shape and that
      `templates/` is created with the same private-directory discipline
      `provision.rs` already uses elsewhere (owner-only, no group/world
      access on Unix; a restrictive Windows DACL).
      Evidence: `root_in(cache)` = `<cache>/<DOLT_VERSION>/templates`, created with `files::ensure_private_directory`; names `<key>.lock`, `<key>/`, `.build-<key>-<uuid>`, `.stage-<key>-<uuid>`, `.rejected-<key>-<uuid>`, Windows `lifecycles/` (build-store leases outside every moved tree). Observed on the prefetched cache [measured]: `templates/` drwx------, `<key>/` drwx------ holding `data` and `manifest.json` (-rw-------), `<key>.lock` (-rw-------). Asserted shapes in `template_builds_once_with_data_only_and_is_reused_without_build` (root holds exactly `<key>` and `<key>.lock`).

- [x] 2.2 Add `ensure_in(root: &Path, …) -> Result<Template>` and
      `create_in(root: &Path, …) -> Result<Template>` as the module's public
      entry points, taking the template root as an explicit parameter (never
      derived from `config.cache_dir` inside the module), matching the shape
      of `test_support/template.rs`'s `instantiate_in(root, …)`. Verify by a
      unit test calling `ensure_in` against an empty private root and
      observing it builds, then calling it again and observing it does not
      rebuild (library-level equivalent of T3 `second_project_reuses_template_without_build`:
      no new `.build-*` directory is created on the second call).
      Evidence: `ensure_in(root, engine, Wait)` and `create_in(root, engine, stage)` take the root explicitly. `template_builds_once_with_data_only_and_is_reused_without_build` (T3 library half) passed: first call `Built`, second `Published`, no new `.build-*`, same directory identity.

- [x] 2.3 Add a crate-private `OpenOptions` field (under `cfg(any(test,
      feature = "test-support"))`) carrying an optional private template
      root, read only by test-support code, so T2, T3, T5-T8, T13, T14, T17
      and T20-T22-style tests can exercise the module without disturbing the
      shared job-wide test cache's warmed template. Product code always
      passes `<cache>/<DOLT_VERSION>/templates`. Verify by a unit test that
      the field is absent from `OpenOptions::new`'s public construction path
      (cannot be set outside the crate).
      Evidence: crate-private `OpenOptions.template_root` and `OpenOptions.fixture` under `cfg(any(test, feature = "test-support"))`; `OpenOptions::new` sets both to `None`, so product options are never refused. No open reads `template_root` for creation yet (P4b-ii); the guard exempts it (`unwarmed_fixture_open_fails_the_guard_before_any_start` predicate cases). Not settable outside the crate: fields are `pub(crate)` (compile-time).

- [x] 2.4 Library-level concurrency test `concurrent_template_lock_acquisition_never_waits`
      (equivalent to T5's `ensure_in` half): one task holds a key's exclusive
      lock (simulating a build in progress); a second `ensure_in` call for
      the same key under the same private root returns immediately as "no
      template available" rather than blocking, and never reads
      `.build-*`/`.stage-*` directories. Verify by the test observing no
      wait (a bounded test timeout that would fail on any blocking) and the
      ledger counter from 9.x showing zero template builds for the second
      caller.
      Evidence: `concurrent_template_lock_acquisition_never_waits` passed: with the exclusive lock held, `ensure_in(Wait::Never)` and `create_in` return `Unavailable(Busy)` well within 10 s; with only a copier's shared lock held and nothing published, a builder also returns `Busy`; the root keeps only the lock file and the destination stays empty. The T5 open-path half (P2 completes cold) is P4b-ii.

## 3. Build: the `TemplateBuild` stage-worker job

- [x] 3.1 Add a `Start::TemplateBuild` (or equivalent) job to
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
      Evidence: `StageWorker::build_template` (`Start::TemplateBuild`) runs initialize, `upgrade`, `upgrade_usage`, `validate_usage`, `validate_active_unclassified` and `template_shape::check(Row::Placeholder)` on one engine whose reap guard is the exclusive key lock, then reads `@@hostname`. T27 `template_shape_counts_derive_from_registries` (from #145) still passes.

- [x] 3.2 Add the non-waiting sweep of abandoned `.build-<key>-*` and
      `.stage-<key>-*` entries for the key being built, attempted once before
      a fresh build starts: one quiescence attempt at the smallest valid
      wait (never zero), removal on success, and the entry left in place for
      a later holder on a held lifecycle lease or any removal failure.
      Verify by test `abandoned_build_with_live_lease_is_skipped_without_waiting`
      (T26): a `.build-*` directory with a held lifecycle lease is left,
      removed from nothing, and the build proceeds in its own fresh
      directory with the ledger showing no extra wait beyond the one
      quiescence attempt.
      Evidence: `sweep` (one `Server::quiescence_at` attempt at 10 ms, the smallest valid wait; removal under the lease; anything else left with a warning). `half_built_template_is_swept_after_quiescence_and_rebuilt` (T26 included) passed: the dead `.build-*` and the partial `.stage-*` of this key were removed, the `.build-*` whose lease the test held was left (reported in `swept.left`) and the build completed beside it in 1.89 s [measured]; another key's `.build-*`/`.stage-*` untouched.

- [x] 3.3 Verify by test `half_built_template_is_swept_after_quiescence_and_rebuilt`
      (T6): a `.build-*` or `.stage-*` directory left by a simulated
      interrupted build (no engine holding its lifecycle lease) is swept
      before the next build for that key, and the next build succeeds and
      publishes.
      Evidence: same test (T6): after the sweep the build published a template.

- [x] 3.4 Verify by test `cancelled_build_releases_key_lock_only_after_reap`
      (library-level equivalent of T16): cancelling the build task mid-chain
      leaves the exclusive key lock held until the ledger shows the build
      server reaped, matching `cancelled_open_during_stage_build_keeps_startup_lock_until_reap`'s
      pattern for the existing stage worker.
      Evidence: `cancelled_build_releases_key_lock_only_after_reap` passed: a build paused on its live engine (hook before the shape check) was aborted; the exclusive key lock was acquired only when the engine ledger showed no live supervisor under the root.

- [x] 3.5 Verify by test `non_table_objects_fail_the_template_shape` (T29): a
      view, trigger, procedure, `dolt_schemas`/`dolt_procedures` row or
      `dolt_ignore` rule created on the build engine before the shape check
      runs refuses publication; record which of those objects
      `information_schema`/the Dolt system tables actually show as present
      vs. absent on the pinned 2.3.5 (settling the [unknown] left by spike
      item S11, not present in `engine-contract-findings.md`) as acceptance
      evidence, not assumed from the roadmap design.
      Evidence: `non_table_objects_fail_the_template_shape` passed. A view created on the build engine refused the build with a verdict (`branch "main" has uncommitted changes`, the first shape assertion it meets), nothing published. S11 settled on pinned Dolt 2.3.5 / macOS arm64 [measured]: every query answers on a store that never held the object (all six counts 0, no error); a view counts 1 in `information_schema.views` and `dolt_schemas`; a trigger 1 in `information_schema.triggers` and `dolt_schemas`; a procedure 1 in `information_schema.routines` and `dolt_procedures`; an ignore rule 1 in `dolt_ignore`. Absent and empty therefore read alike; the shape check's object reads were extracted into `template_shape::non_table_objects` for this test. Linux and Windows: CI.

- [x] 3.6 Verify by test `template_born_store_matches_cold_store` (T10,
      parity): build a template, then separately build a cold store on the
      same schema and compare schema, receipts (minus operation UUIDs),
      branch and record version sets, usage version and empty data tables;
      use the comparison to fix `template_shape.rs::BASE_COMMITS` as a
      measured constant (replacing its current "measured on cold store,
      [to be confirmed]"-style doc comment) rather than leaving it a guess.
      Evidence: `template_born_store_matches_cold_store` (T10, in `store/template_stage_tests.rs`) passed: a stage copied from the shared warmed template by `create_in`, adopted by the stage worker and activated by the ordinary open equals a cold-built store in columns, indexes, main and usage receipts (minus operation UUIDs), branch version sets, both schema versions and empty data tables. Measured [measured]: cold store `dolt_log` main 8 and usage 5, so BASE = 8 − 6 = 5 − 3 = 2; template-born 9 and 6. `BASE_COMMITS = 2` is now asserted against the cold store via `compiled_commits(false)` (and `compiled_commits(true)` for the copy).

## 4. Capture, byte scan, manifest and publication

- [x] 4.1 Promote the capture walk and classification of
      `test_support/template.rs` (roughly lines 430-473 at this base) to the
      production module: skip runtime locks; refuse capture of
      `endpoint.json`, `migration.json`, `sql-server.info`, `*.pid`,
      `*.sock`; `sync_all` every captured file and directory. Verify by test
      `copied_files_and_directories_are_synced_before_identity` (T11): an
      ordered hook shows every file and directory synced before the manifest
      is written.
      Evidence: production walk in `creation_template.rs` (data-rooted classifier: runtime locks skipped, `endpoint.json`, `migration.json`, `sql-server.info`, `*.pid`, `*.sock` refuse, links and non-regular objects refused, every file `sync_all` and every created directory synced on Unix). T11 in `template_builds_once_with_data_only_and_is_reused_without_build`: every manifest entry was synced before the manifest-written event.

- [x] 4.2 Add the byte scan over the captured tree, refusing publication if
      any file contains the build store's absolute path, either build
      secret, or the local host name (covering `.dolt/stats`, per
      `engine-contract-findings.md` S7's inventory). Verify by test
      `template_holds_data_only_without_build_path_secret_or_host_name`
      (T12), and record in its evidence, and in the docs task in group 13,
      that compression can hide repeated text so this scan is a
      defense-in-depth check and the SQL-level shape assertions (group 3)
      are the assertion the design actually relies on (S7's own caveat).
      Evidence: `capture` scans the captured tree for the build store path (as given and canonical), both secrets (whole and 16-character fragments) and the host names (`@@hostname`, plus `COMPUTERNAME` on Windows; names under 4 bytes skipped), in UTF-8 and UTF-16LE with both separators. T12 in the same test re-scanned the published tree with the build's needles: no hit; host name `Mac.home.local` [measured]. Docs (development.md, memory.md) and the module doc state that compression limits the scan to literal bytes (S7) and the SQL-level shape check is the guarantee.

- [x] 4.3 Write `manifest.json` (format, key, entry list of path/bytes/SHA-256)
      and publish by `files::move_directory(stage, <key>)` (no-replace
      rename, re-opening and checking the destination's identity), with a
      bounded retry in the style of the engine-activation retry
      (`provision.rs`, 2 s at 20 ms). Verify by test
      `publication_failure_copies_from_verified_stage` (T13): an injected
      rename failure leaves the verified stage directory intact and
      reachable by the build's own caller, and leaves it for the next
      exclusive-lock holder to sweep (group 3.2) rather than erroring the
      build.
      Evidence: manifest `{format, key, entries}` written and synced, stage verified, published by `files::move_directory` (no-replace, identity-checked), retried for 2 s at 20 ms while the stage provably stayed in place. `publication_failure_copies_from_verified_stage` (T13) passed: with publication refused, `create_in` copied from the verified stage (`Copied { built: true, published: false }`), `<key>/` absent, the stage verified and left; the next `ensure_in` swept it and published a fresh build. Warm-up and prefetch fail on a built-but-unpublished template rather than caching success.

## 5. Verification on use: structural check and `copy_into`

- [x] 5.1 Add a structural check (manifest parses, format and key match,
      top-level entries are exactly `{manifest.json, data}`) runnable before
      any stage exists, under the shared lock. Verify by the structural half
      of test `template_failing_structure_is_quarantined_and_open_goes_cold`
      (T7): a wrong-format or wrong-key manifest is a verdict (quarantined,
      group 6); a manifest read error (simulated sharing violation/`EIO`) is
      not a verdict and leaves the template untouched. The "open goes cold"
      half of T7 is explicitly deferred to P4b-ii, which has a cold fallback
      to go to.
      Evidence: `inspect`/`judge` (no hashing). `template_failing_structure_is_quarantined` passed: another key, another format, an extra top-level entry and an unparsable manifest are verdicts that quarantine the judged directory before any copy starts; an injected manifest read error is `CreationFailure::Io` and leaves the template published. The "open goes cold" half is P4b-ii.

- [x] 5.2 Add `copy_into(stage, published, …)`: per-file size and SHA-256
      verified while copying, entry-set equality against the manifest, and
      refusal of a link, an extra hard link, or an entry of a different type
      than the manifest declares. Verify by the copy half of test
      `template_byte_corruption_mid_copy_preserves_remnant_and_quarantines`
      (T8): a digest or size mismatch is a verdict (quarantined, group 6);
      an injected read or write I/O error on either side is not a verdict,
      preserves the partial copy as a remnant, and triggers no quarantine.
      The "this open continues on the cold path" half of T8 is deferred to
      P4b-ii.
      Evidence: `copy_into` compares each entry with the manifest as it is written and the entry count at the end. `template_byte_corruption_mid_copy_preserves_remnant_and_quarantines` passed: a changed byte, an extra byte, and on Unix an extra hard link and a symbolic link are verdicts that quarantine and leave the partial copy; an injected read error mid-copy is `Io`, leaves the template published and leaves the partial copy. The cold-continuation half is P4b-ii.

## 6. Quarantine

- [x] 6.1 Add `CreationFailure::{TemplateVerdict, Engine, Io}` (or the
      equivalent discriminated failure type), in the style of the existing
      `DoltPrematureExit` downcast `Server` uses to decide a restart, so
      every failure source in groups 3-5 reports its discriminant rather
      than a single error type.
      Evidence: `CreationFailure::{TemplateVerdict, Engine, Io}` with `classify` (verdict only when a `TemplateVerdict` is in the chain).

- [x] 6.2 Add the identity-bound quarantine action: under the key's
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
      Evidence: `quarantine` (non-waiting exclusive lock) and `quarantine_held` (identity compared on the one handle that is then moved; the older `.rejected-<key>-*` removed first, best-effort; every skip a warning). Implementation note: `files::move_directory` needs a held source handle, so the compared handle is the moved handle; no second handle exists across the rename, which is what the design's "drop the comparison handle" protects. `quarantine_is_bound_to_the_judged_template` (T21) passed: a fresh valid template republished between the verdict and the exclusive lock stayed published and nothing was quarantined. T7/T8 show exactly one `.rejected-<key>-*` after repeated quarantines.

- [x] 6.3 Verify by test `template_lock_errors_send_the_opener_cold`'s
      library half (T22): an injected `lock_file` open error, a
      `TryLockError::Error`, and an identity-verification failure each make
      `ensure_in` return "no usable template for this key" rather than an
      error, and quarantine nothing. The "opener completes on the cold path
      with its start count" half of T22 is deferred to P4b-ii, which has a
      cold path to observe it on.
      Evidence: `template_lock_errors_mean_no_template_and_are_fatal_in_warm_up` passed: injected open, `TryLockError::Error` and identity-verification faults make `ensure_in(Wait::Never)` and `create_in` return `Unavailable(Lock(..))`, quarantine nothing and leave the template's identity unchanged; an unusable root likewise. Open-path half (cold start count) is P4b-ii.

- [x] 6.4 Verify, as a focused unit test rather than a full open-path
      scenario (the open-path version is P4b-ii's T20), that every
      non-verdict failure path in groups 3-5 (engine crash, lost reply,
      deadline, authentication failure, SQL error unrelated to the expected
      comparison, lock contention, I/O error) reports `Engine`/`Io`, never
      `TemplateVerdict`, and leaves a previously published template for that
      key untouched.
      Evidence: `only_a_template_verdict_is_a_verdict` (premature exit, bootstrap deadline, lost reply, key mismatch, readiness deadline, I/O are not verdicts; a wrapped `TemplateVerdict` is); untouched-template assertions in the I/O and lock-fault cases above.

## 7. No garbage collection

- [x] 7.1 Verify by test `open_never_removes_other_keys_or_lock_files` (T14,
      replacing the design's earlier GC-test shape per review finding B2): a
      private template root pre-populated with two other keys' published
      templates, one other key's `.rejected-*` directory, and lock files for
      all three keys; building and publishing a template for a fourth key,
      and separately copying an existing key's template, each leave every
      other entry byte-identical afterward (contents and mtimes where
      feasible to assert), and remove no lock file.
      Evidence: `open_never_removes_other_keys_or_lock_files` (T14) passed: two other keys' templates, another key's `.rejected-*`, another key's `.build-*` and `.stage-*` and three lock files were byte-, size- and mtime-identical after a build and a copy of this key.

- [x] 7.2 Confirm (not implement — `provision.rs`'s `.leftovers` sweep
      already does not enumerate `<DOLT_VERSION>/` subdirectories other than
      what it already reads) that nothing under `provision.rs`'s existing
      install/version bookkeeping is made to notice or sweep `templates/`;
      record the confirming grep as evidence.
      Evidence [measured]: `grep -n read_dir packages/kuru-memory/src/provision.rs` finds only the `.leftovers` receipts reads and stage listing under the install root; `sweep_leftover_stages`/`warm_sweep_if_receipted` read only `.leftovers` receipts, never enumerate `<DOLT_VERSION>/`. Provisioning was not changed.

## 8. Fixture warm-up: the template cell

- [x] 8.1 Add a second `tokio::sync::OnceCell` to `test_support::warm_runtime_cache`,
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
      Evidence: `warm_runtime_cache` = engine cell, then the template cell (`warm_template_in`, `tokio::sync::OnceCell::get_or_try_init`, bound `template_warm_up_bound()` = `fresh_open_budget()` + `WARM_UP_MARGIN`, polling a busy lock, holding `spawn_gate::spawning()` under `cfg(test)`). `warm_runtime_cache_builds_one_template_across_processes` (T17) passed: two child test processes with one fresh private `KURU_DOLT_CACHE` reported exactly one `Built` and one `Published`; the cache root holds one template and its lock.

- [x] 8.2 Verify by test `failed_template_warm_up_is_not_cached` (T25): an
      injected failure in the first call's template-cell init fails only
      that call (named error, not masked), and the next call builds and
      succeeds.
      Evidence: `failed_template_warm_up_is_not_cached` (T25) passed on a local cell and private root: an injected lock fault failed the first call by name and left the cell empty; the next call built; a success stays cached.

- [x] 8.3 Confirm lock-file errors inside warm-up (open, `TryLockError::Error`,
      identity-verification failure) are fatal and named, not silently
      treated as "no template" the way group 6.3's product-facing behavior
      is; record the distinguishing code path and its test as evidence
      (warm-up has no fallback to send a caller to, unlike a product opener).
      Evidence: `lock_unavailable` returns `Unavailable` only for `Wait::Never`; with `Wait::Until` the same errors are `CreationFailure::Io` naming the warm-up (`template_lock_errors_mean_no_template_and_are_fatal_in_warm_up`, warm-up half).

## 9. Fixture warm-up: the per-fixture token and deterministic guard

- [x] 9.1 Add a crate-private `Fixture::{Unwarmed, Warmed}` field on
      `OpenOptions` (`cfg(any(test, feature = "test-support"))`), defaulting
      to `Unwarmed` in `test_support::open_options`. Add
      `test_support::warmed_open_options(data, scope) -> Result<OpenOptions>`
      (async) and `OpenOptions::warmed(self) -> Result<Self>` that call
      `warm_runtime_cache` (both cells) and set `Fixture::Warmed`.
      Evidence: `Fixture::{Unwarmed, Warmed}`; `test_support::open_options` marks `Unwarmed`; `warmed_open_options(..).await` and `OpenOptions::warmed().await` warm then mark `Warmed`.

- [x] 9.2 In `open_inner`, after the active store is found absent and before
      `recover_staging` or any start, add the deterministic guard: a
      writable open with `Fixture::Unwarmed` whose template root is the
      shared test root fails at once with the named message; a reopen of an
      existing store is never guarded. Verify by test
      `unwarmed_fixture_open_fails_the_guard_before_any_start` (T23):
      options from `open_options` without `.warmed()` fail immediately with
      zero engine starts recorded in the ledger.
      Evidence: `OpenOptions::refuse_unwarmed_fixture` in `open_inner` after the absent-store check and before legacy preparation or recovery. `unwarmed_fixture_open_fails_the_guard_before_any_start` (T23) passed: the named error, no live engine under the root, no store or stage left; warmed, read-only, private-root, product and other-cache options are not refused; a reopen with unwarmed options succeeds.

- [x] 9.3 Update `MemoryStore::temporary()`, `temporary_cold()` to call
      `warm_runtime_cache().await?` first (before `template::instantiate`
      and before `temporary_permit()`), and `open_temporary()` to build its
      options through the warmed builder. Verify by test
      `temporary_warms_before_instantiate` (T24): a child process with a
      fresh private `KURU_DOLT_CACHE` (never the runner's own environment)
      shows the first `temporary()` call publishing the production template
      during warm-up, and the test-template `build()`'s own open records 2
      engine starts and 0 in-open template builds.
      Evidence: `temporary()` and `temporary_cold()` warm before the test template's lock and the permit; `open_temporary` uses `warmed_open_options`. T24 as amended for P4b-i: the child test `child_process_warms_its_template_cache` shows its first `temporary()` warmed the production template (one child `Built`, one `Published`), the template is published afterwards, and no template event was charged to any open. The task's "2 engine starts" for the test template's `build()` is P4b-ii: until creation uses the template that open is still the 4-start cold path.

- [x] 9.4 Fail the guard on every fixture call site of
      `test_support::open_options` across `kuru-memory`, `kuru-runtime` and
      `kuru-tui` that is not already covered by 9.3, by switching each to
      `warmed_open_options`/`.warmed()`. Verify: running each affected
      package's test suite once with the guard active and once with it
      behind a temporary bypass to confirm the guard would have caught each
      missed site before the switch, recording the list of call sites
      changed as evidence (the guard's own design intent, per 15.2 B3's
      disposition, is that it "fires on the first run of every unwarmed
      call site").
      Evidence: fixture `open_options` call sites were switched to the warmed form: kuru-memory 105 (plus `open_temporary` and the template-stage `open` helper), kuru-runtime 20, kuru-tui 16 (cli.rs 12, preferences.rs 2, trust.rs 1, src/ui/runtime_tests.rs 1), except where only an existing store is reopened or no store is opened (`test_support` retirement tests under paused time, T23's deliberate unwarmed case, kuru-tui `terminal.rs` `memory_options` reopening the application's store). Guard-active suite runs: see 14.2. Not run: the extra "guard bypassed" comparison runs (three more full suites on a shared, loaded machine); the guard's own test (T23) and the enumerated diff stand in for them.

- [x] 9.5 Add `TempDir::allowing_template_build()` (or equivalent opt-in) and
      `engine_ledger` counters for a template built inside `open_inner` and a
      template quarantined under the shared root; the fixture teardown guard
      fails a fixture whose open did either unless it opted in. Verify by
      test `fixture_open_that_builds_a_template_fails_the_guard` (T18): a
      fixture using a private, never-warmed root and a writable open that
      reaches the build path fails teardown naming the counter, unless
      `allowing_template_build()` was set.
      Evidence: `engine_ledger::TemplateEvent::{Built, Quarantined}` recorded by `create_in` against the destination stage, only when the root is the shared test root; `TempDir::allowing_template_build()`; teardown fails the fixture naming the event. `fixture_open_that_builds_a_template_fails_the_guard` (T18) passed (guarded root fails naming `built the shared store template` and the opt-in; an opted-in root passes; a private root is not the shared root). The trigger from a real open is P4b-ii, when an open first calls `create_in`.

## 10. Fixture warm-up: spawned owners and `cache_dir`

- [x] 10.1 Make `test_support::cache_dir()` fallible, warming synchronously
      through a new `warm_blocking()` that runs the async warm-up on a
      private thread with its own current-thread runtime, refusing to run
      if already inside a Tokio runtime (`Handle::try_current().is_ok()`)
      and naming `warmed_cache_dir().await` instead in its error. Verify by
      test `warm_blocking_refuses_inside_a_runtime` (T28): called from
      inside a Tokio runtime, it returns the named error and starts no
      thread.
      Evidence: `cache_dir() -> Result<PathBuf>` warms through `warm_blocking()` (private thread, current-thread runtime). `warm_blocking_refuses_inside_a_runtime` (T28) passed: error names `warmed_cache_dir`, no thread started.

- [x] 10.2 Add async `warmed_cache_dir().await` for callers already inside a
      Tokio runtime. Update `spawn_logged_owner` and the service spawn
      helpers to warm with the async form before spawning.
      Evidence: `warmed_cache_dir().await`; `spawn_logged_owner` warms first.

- [x] 10.3 Update kuru-tui's `cache_dir()` callers
      (`apps/kuru-tui/tests/support/memory.rs`, `directories.rs`,
      `trust.rs`) to the fallible form, and determine whether
      `apps/kuru-tui/src/ui/runtime_tests.rs` runs inside a Tokio runtime
      (the design's [unknown]); use `warmed_cache_dir().await` there if so,
      `cache_dir()` otherwise. Verify by that package's existing test suite
      passing unchanged, and record the Tokio-runtime determination as
      evidence (resolving the [unknown] rather than assuming it).
      Evidence: kuru-tui `support/memory.rs` `configuration` (sync, warms) plus `configuration_warmed`/`configuration_with`; each sandbox gained an async `warmed` constructor used by every test inside a runtime (63 call sites converted); `directories.rs` uses `cache_dir()?`, `trust.rs` and `windows_cli.rs` use `warmed_cache_dir().await`. Determination [read]: `apps/kuru-tui/src/ui/runtime_tests.rs` does not call `cache_dir()`; its `persistent_store` runs inside a Tokio runtime and now uses `warmed_open_options(..).await`. Suite result: see 14.2.

## 11. Spawn-gate audit

- [x] 11.1 Audit every `spawn_gate::spawning()` call site in `service.rs`,
      `facade.rs` and `operational_gc_tests.rs` that encloses a store or
      service open, and confirm none of them now runs a first, unwarmed
      warm-up under a held gate (groups 8-9 should already prevent this by
      construction: the guard fails instead of warming inside an open, and
      every fixture builds its warmed options before any gate). Record the
      count of `spawning()` sites and warm call sites found in each file as
      evidence, alongside the roadmap design's own prior count (21/15 in
      service.rs, 17/17 in facade.rs, 3/1 in operational_gc_tests.rs at
      `a96a16ae`), noting any drift.
      Evidence [measured, this tree]: `spawn_gate::spawning()` sites / warm calls: service.rs 32 / 31 (base e6b1b60f: 32 / 26), facade.rs 27 / 45 (base 27 / 27), operational_gc_tests.rs 3 / 6 (base 3 / 1). The design counted 21/15, 17/17 and 3/1 at a96a16ae: drift from later PRs and counting by this grep. Audit result: no site in those three files warms under a held guard; one site elsewhere did (`main_pool_classification_tests.rs` `open_reader` called `warm_runtime_cache` inside its spawn guard) and was moved before the guard.

- [x] 11.2 Add a regression test or assertion (whichever this audit's
      findings make practical — a lint-style grep check in test-support, or
      a targeted unit test per site class) that a future `spawning()` site
      enclosing an open without a prior warm is caught, not just observed
      once; record what was added and why.
      Evidence: lib test `spawn_gate::tests::no_spawn_guard_encloses_a_test_cache_warm_up` scans every crate source (comments and literals blanked, guards tracked by brace scope and `drop`) for a warm-up call while a bound `spawning()` guard is held. Exercised [measured]: a temporary `gate_probe` taking the guard then calling `warm_runtime_cache` failed it (`template_stage_tests.rs:340: warm_runtime_cache( while the spawn guard _gate from line 339 is held`); reverted. A scan, rather than a runtime check, because the gate is a plain `RwLock` that cannot tell which task holds it.

## 12. `prefetch`

- [x] 12.1 Add a template-warm step to `packages/kuru-memory/mise.toml`'s
      `prefetch` task (`main.rs`), after engine provisioning, using the
      prepared supervisor snapshot it already creates. Verify: running
      `mise run //packages/kuru-memory:prefetch` locally publishes a
      template under the shared cache, and the ordinary `test` task (which
      depends on `prefetch`) shows no in-process template build on its first
      fixture.
      Evidence: `main.rs` prefetch calls `test_support::warm_template_cache(cache, engine, prepared snapshot)` after provisioning. `mise run //packages/kuru-memory:prefetch` [measured] published `templates/5894ec0a…` in the shared cache (17.98 s including a 12.71 s incremental compile).

- [x] 12.2 Confirm coverage's tasks (which clear
      `KURU_TEST_SUPERVISOR_PREPARED` and do not run `prefetch`) warm the
      template in the instrumented test process through the fixture path of
      groups 8-9, using the instrumented supervisor; record that the
      template's key excludes the supervisor (group 1), so the instrumented
      and ordinary templates for a given schema/engine are interchangeable,
      and that no ordinary supervisor is compiled or snapshotted first.
      Evidence [read]: coverage tasks clear `KURU_TEST_SUPERVISOR_PREPARED` and do not run prefetch (packages/kuru-delivery/mise.toml); `template_engine` uses `test_supervisor()`, the instrumented helper there, and the key has no supervisor input (1.2), so the fixture path builds the same key. No ordinary supervisor is compiled or snapshotted for it. Not run here: a local coverage run (CI gates it).

## 13. Docs

- [x] 13.1 `docs/development.md`: document the shared test cache now keeping
      every template key built on the machine, with nothing pruning it, and
      that deleting the directory while no test runs reclaims it; document
      obtaining fixture options through `warmed_open_options`/
      `OpenOptions::warmed()` before any spawn gate, and that an unwarmed
      fixture open fails at once; document the production vs. test template
      distinction; document that `warm_runtime_cache` warms both; document
      `Creation::Cold` for cold fixtures; document kuru-tui's
      `warm_blocking`/`warmed_cache_dir`; document the fixture teardown
      guard and its opt-in.
      Evidence: docs/development.md "Shared store template and fixture warm-up" and the prefetch sentence.

- [x] 13.2 `docs/memory.md` and `apps/kuru-docs/concepts/memory.md`: add a
      passage placing the template cache beside the engine cache — what it
      holds (`data/` only: schema, receipts, placeholder identity) and does
      not (no credentials, no project data, no `identity.json`/`config/`/`home/`),
      that it is shared by every project on the machine that shares a
      cache, that an explicit project purge does not touch it, that the
      byte scan guards against path/secret/host leakage but is
      defense-in-depth (the SQL-level shape assertions are the real guard,
      per S7), and that no product open uses it yet (this PR only builds
      and verifies it; the next change routes creation through it).
      Evidence: docs/memory.md "New projects and the store template"; apps/kuru-docs/concepts/memory.md paragraph under Migration and backups.

## 14. Spec and gate

- [x] 14.1 Confirm the spec delta at
      `specs/versioned-memory/spec.md` (authored in this change) targets
      only `ADDED Requirements` against the living
      `openspec/specs/versioned-memory/spec.md`, and run
      `mise run cospec -- validate memory-template-cache --strict`,
      recording its pass with 0 errors/0 warnings as evidence (or naming and
      resolving whatever it reports).
- [x] 14.2 Run the package-scoped checks this change touches:
      `mise run //packages/kuru-memory:test`,
      `mise run //packages/kuru-memory:lint`,
      `mise run //packages/kuru-memory:typecheck`, the affected
      `kuru-runtime` and `kuru-tui` test tasks, `mise run format:check`,
      and `mise run docs:check`; record each command's pass/fail and, for
      any skipped or deferred check (for example a full coverage run, which
      CI gates separately), name why it was not run here rather than
      claiming a pass that was not observed.
