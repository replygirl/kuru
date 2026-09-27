## 1. Acceptance evidence (defined before implementation)

- [x] 1.1 Record the before baseline: wall time for `mise run //packages/kuru-memory:test` and `mise run //packages/kuru-runtime:test`, per-package line coverage from one `mise run coverage`, and the `RUST_TEST_THREADS` history. Verify the numbers are written in this file.
- [ ] 1.2 After the default flips, run the full `kuru-memory`, `kuru-runtime` and `kuru-tui` suites locally. Verify that every suite passes with no test removed or ignored.
- [ ] 1.3 Per-package line coverage after the change stays within noise of the baseline, and the 90% workspace gate holds. Verify by reporting both sets of numbers here.
- [ ] 1.4 Windows is verified by native CI on the PR head, not assumed: the copied tree passes `Directory`/`LifecycleLease` private-object and DACL validation. Verify the run ID is recorded.
- [ ] 1.5 The PR's own CI run measures the per-binary saving on each native OS. Verify the timings are recorded against the baseline.

### Observed baseline (before, macOS arm64, base `8225613d`)

- `RUST_TEST_THREADS=2` came from `8e621705` (#11, the Dolt bundling change). It then
  spread to the coverage shards in `86a4665c` (#19) and to delivery `test:install` in
  `67448442` (#88). None of those commit messages gives a reason for choosing 2.
- `mise run //packages/kuru-memory:test`: exit 0, 660 s wall including build. The lib
  ran 278 tests in 568.82 s; the integration targets ran 5 in 13.28 s, 12 in 19.82 s,
  1 in 3.12 s and 6 in 0.01 s.
- `mise run //packages/kuru-runtime:test`: exit 0, 659 s wall. The lib ran 220 tests in
  603.14 s.
- `mise run coverage`: exit 0, 1637 s wall, 87,523/93,264 lines (93.84%). By package:
  kuru-memory 94.63%, kuru-runtime 96.14%, kuru-tui 93.83%, kuru-connectors 94.90%,
  kuru-core 96.45%, kuru-platform 88.28%, kuru-archive 98.29%, kuru-delivery 84.64%.
  The per-file summary is kept in local scratch (`template-fixture-coverage-before.txt`).

## 2. Template creation under lock (T1)

- [ ] 2.1 In `packages/kuru-memory/src/test_support.rs` (or `test_support/template.rs`), derive the fingerprint from the SHA-256 of the supervisor `test_supervisor()` returns plus the current schema version. Verify that a unit test shows a changed supervisor or schema version yields a different key.
- [ ] 2.2 Create the template in the private profile directory beside `kuru-test-supervisors`, under a `lock_file` lease, from one cold `temporary_cold()`-style open followed by `close()`, staged in `PrivateTemp` and published atomically. Verify with a test that concurrent processes produce one template and a second process reuses it.
- [ ] 2.3 Revalidate an existing template's private objects on reuse, and discard and rebuild an invalid or incomplete one under the lock. Verify that a test corrupts a template and observes the rebuild.

## 3. Private copy excluding runtime files (T2)

- [ ] 3.1 Open and cleanly close one store, list the retained tree and fix an explicit allowlist. Lock, lease, PID, socket, endpoint and Dolt server lock/info files are excluded. Verify with a test asserting the exact copied set.
- [ ] 3.2 Copy per file through `Directory`/`seal_private`, rejecting symlinks and hard links, so owner-only modes and Windows DACLs are recreated. Verify with a test that a link in the template is refused and that the copy passes the store's private-object validation.

## 4. Default template API with cold opt-out (T3)

- [ ] 4.1 Make `MemoryStore::temporary()` (test-support gated, `packages/kuru-memory/src/store.rs`) copy the template and run the unchanged `open_inner`, and add the explicit cold API. Verify that no non-gated code changes, using `git diff` scoped to cfg-gated items.
- [ ] 4.2 Enumerate every `temporary()` caller in `packages/kuru-memory` (lib, `tests/memory.rs`) and tag each as 1a/1b/2/3/identity. Move class-2, class-3 and identity/secret/migration-asserting tests to the cold API. Verify that the tag list is recorded here.
- [ ] 4.3 Add an explicit cold-path test in `kuru-memory` that bypasses the template and asserts a fresh staged open. Verify that it passes.

## 5. Runtime and TUI call sites (T4)

- [ ] 5.1 Scan `packages/kuru-runtime` and `apps/kuru-tui` for tests that compare `instance()`, secrets or migration state across two temporary stores. Move any hits to the cold API. Verify that the scan result is recorded here.
- [ ] 5.2 Run the `kuru-runtime` and `kuru-tui` suites with the template default. Verify that they pass.

## 6. Measurements (T5)

- [ ] 6.1 Re-run the memory and runtime suites locally with the same settings as 1.1 and record the after wall times. Verify the comparison is recorded here.
- [ ] 6.2 Run `mise run coverage` once, with no concurrent coverage writer, and record per-package coverage for 1.3.
- [ ] 6.3 Validate strictly and archive the change before the final branch commit.
