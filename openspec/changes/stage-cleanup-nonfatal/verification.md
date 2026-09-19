## 1. Publication survives a failed stage cleanup [critical]

- [x] 1.1 @regression (agent) force bounded cleanup exhaustion through the `#[cfg(test)]` removal-failure hook on a host cold provision -> `provision::tests::exhausted_stage_cleanup_publishes_the_engine_and_receipts_the_retained_stage` passed 3/3 on macOS: the open returns the published binary and `verify_version` runs it.
- [x] 1.2 @unit (agent) inspect the leftover after that forced exhaustion -> the same fixture asserts the `.install-*` stage is still on disk with its probe copy, and the owner-only `versions/.leftovers/<stage>.json` records relative paths, first cause with `(os error 145)`, 88 attempts, 2003 ms, the publication digest, target, engine version and `published: true`; `files::tests::a_failed_stage_cleanup_retains_the_stage_and_names_its_cause` asserts the retained identity and typed exhaustion.
- [x] 1.3 @unit (agent) exercise the rejected and uncertain publication paths unchanged -> the whole package suite passed (159 lib tests, `mise run //packages/kuru-memory:test` green), including `rejected_activation_preserves_verified_stage_and_occupied_destination` and `activation_source_open_failure_preserves_stage_before_releasing_cache_lock`; only `finish_published` changed, and the `retain` paths and the 2 s bound are untouched.
- [~] 1.4 @regression (agent) run the Windows-native variant using the existing delete-pending fixture shape from `packages/kuru-memory/src/files.rs` tests -> defer: Windows native execution is pending. `provision::native_tests::published_engine_retains_its_failed_stage_cleanup_and_releases_cache_lease` now asserts the retained-stage report instead of an error, and `cargo check --target x86_64-pc-windows-msvc -p kuru-memory` cannot run on this macOS host (`libsqlite3-sys` fails in `cc-rs`), so the Windows-gated code is review-verified only.

## 2. Collection

- [x] 2.1 @integration (agent) open again after a receipted leftover exists -> the sweep removes the stage and its receipt under the installation lock. `provision::tests::cold_provision_sweeps_a_receipted_stage_before_creating_a_new_one` (cold path, lock held before `PrivateTemp::new`) and the extended `exhausted_stage_cleanup_publishes_the_engine_and_receipts_the_retained_stage` (warm path, follow-up open) both passed 3/3 on macOS.
- [x] 2.2 @unit (agent) make the swept removal fail -> stage and receipt both remain. `provision::tests::sweep_leaves_a_stage_and_its_receipt_when_removal_is_rejected` forces a real `RemovalError` via a symlink descendant (the same cross-platform trick `fs.rs`'s `checked_tree_removal_rejects_a_symlink_descendant` already uses), taking the identical `Err(_) => outcome.remaining += 1` arm the sweep gives an `Uncertain` result; passed 3/3. A literal native `Uncertain` phase remains Windows-only and is review-verified (see 1.4/4.2), same as `RemovalError`'s existing Rejected/Uncertain split in `fs.rs`.
- [x] 2.3 @unit (agent) place an `.install-*` directory with no receipt beside a receipted one -> only the receipted stage is removed. `provision::tests::sweep_collects_only_receipted_stages` passed 3/3.
- [x] 2.4 @integration (agent) warm-open with no `versions/.leftovers` while another process holds the installation lock -> the open completes without acquiring or waiting on the lock. Pre-existing `provision::tests::warm_verification_does_not_wait_for_the_installation_lock` still passes unchanged; the new `provision::tests::warm_open_skips_the_sweep_when_the_installation_lock_is_busy` covers the receipts-present variant (a busy lock is skipped, not waited on, and the receipt survives for the next attempt) and passed 3/3.
- [x] 2.5 @unit (agent) exceed `LEFTOVER_STAGE_CAP` receipts -> the count is reported and nothing additional is deleted. `provision::tests::sweep_reports_the_cap_without_deleting_any_uncollectable_stage` passed 3/3: `SweepOutcome::reached_cap` is set and every stage/receipt is left in place.

## 3. Surfacing and documentation

- [ ] 3.1 @e2e (agent) run a CLI command whose open retains a stage -> exactly one `Memory:` notice line on stderr after `Memory: ready.`, and `run --json` stdout parses as JSON.
- [ ] 3.2 @unit (agent) inspect the diagnostics ring after a retained stage -> one structured warn record with the typed report fields; no memory write and no model-visible surface.
- [ ] 3.3 @unit (agent) run the progress-stage tests -> every stage including the new one reports exactly once through the widened bitmask.
- [ ] 3.4 @unit (agent) run docs build and link/content checks -> `docs/memory.md` and `apps/kuru-docs/concepts/memory.md` state leftover install stages and their collection, and checks pass.

## 4. Static and review

- [ ] 4.1 @unit (agent) run scoped memory and TUI format, lint and typecheck -> all pass on the host with the verified bundle mirror.
- [ ] 4.2 @manual (agent) independently review publication exactness, the unchanged two-second bound, the no-delete-on-uncertainty rule and lock discipline -> two independent reviews clear the final source.
