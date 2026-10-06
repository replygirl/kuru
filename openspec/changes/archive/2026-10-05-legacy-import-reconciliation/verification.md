# Verification

## 1. Provider-free inventory and source preservation [critical]

- [x] 1.1 @integration (agent) inventory multiple isolated legacy scopes from one bounded readonly SQLite view, including committed WAL data -> focused memory selections completed 12/12 after fixture corrections; real CLI `cli_inventories_and_explicitly_imports_one_moved_legacy_scope` passed 1/1 and confirmed unchanged DB/WAL bytes, no private-row output, and no memory activation during inventory
- [x] 1.2 @integration (agent) inventory malformed, changing, over-bound, and ambiguous sources -> focused memory selections completed 12/12 after fixture corrections; typed bounded refusal cases returned no partial rows or published snapshots
- [x] 1.3 @regression (agent) exercise SQLite backup progress beyond the old whole-copy limit and a true stalled/erroring source -> focused memory selections completed 12/12 after fixture corrections; progress beyond 30 seconds completed, no-progress stalled, and backup-step error cases passed

## 2. Explicit import mapping and no-effect refusal [critical]

- [x] 2.1 @integration (agent) import one exact scope into an absent canonical target with ordered transcript rows and opaque state -> focused memory selections completed 12/12 after fixture corrections; exact ordered content/state and durable receipt were verified with source and snapshot retained
- [x] 2.2 @integration (agent) import an explicitly selected source scope into another absent canonical target -> focused memory selections completed 12/12 after fixture corrections; the real CLI fixture passed 1/1 with only the selected leading scope remapped and receipt source/target scopes verified
- [x] 2.3 @integration (agent) exercise unselected/ambiguous family, unsupported identity payload, scope-boundary row, transformed collision, existing destination, and suppression -> focused memory selections completed 12/12 after fixture corrections; typed refusals left no candidate/snapshot or target activation; the real CLI filter verified unselected refusal and replay refusal
- [x] 2.4 @regression (agent) exercise ordinary automatic first-open import beside a row with a non-delimited scope prefix -> focused memory selections completed 12/12 after fixture corrections; exact-scope skip behavior passed

## 3. N4 admission and cancellation-safe ownership [critical]

- [x] 3.1 @integration (agent) attempt import while a real session is admitted and verify the N4 native draining barrier -> `explicit_import_refuses_real_n4_driver_before_staging` passed 1/1 on the final source; `session_native_barriers_exclude_same_driver_and_maintenance` passed 1/1 in the accepted focused memory selections. The import refused before staging for an admitted session; the underlying N4 fixture exercised native draining refusal/maintenance exclusion. No separate importer-during-transition fixture was run
- [x] 3.2 @integration (agent) drop the import caller after owned work begins and observe actual local close and service cleanup/reap -> final-source `cancelled_import_retains_maintenance_through_close_and_reap` passed 1/1; the caller was dropped while the real owner was open, and maintenance remained held until local close/reap. Forced close-error behavior remains source-reviewed, not fixture-proven
- [x] 3.3 @unit (agent) distinguish native WouldBlock from other operating-system lock errors -> `maintenance_lock_error_distinguishes_contention_from_io` passed 1/1 in the accepted focused memory selections; only `WouldBlock` maps to the typed busy refusal and permission I/O retains its `io::Error` cause

## 4. CLI and documentation

- [x] 4.1 @e2e (agent) run the real `kuru memory inventory` and explicit `kuru memory import` commands against isolated fake legacy and Kuru stores -> `cli_inventories_and_explicitly_imports_one_moved_legacy_scope` passed 1/1; refusal kinds and bounded receipt were observable without private rows, and DB/WAL bytes were preserved
- [x] 4.2 @regression (agent) run owning memory/app host and Windows lint/typecheck, formatter, docs build/content/link checks, managed drift, and Cospec strict/apply -> memory host/Windows lint and typecheck, TUI host/Windows lint and typecheck, format check, docs:check, cospec:managed:check, strict validation (0 errors/0 warnings), and apply gate (clear, no blockers) all exited 0. Normal commit hooks run after archiving
- [x] 4.3 @runtime (agent) complete local evidence and verify archive readiness -> implemented tasks and local checks are complete; strict validation and apply are clear. Actual archive follows this ledger update. Hosted native platform, full 90% coverage, and release acceptance remain deferred to the published exact commit and are required before merge/release completion
- [~] 4.4 @integration (agent) exercise an import during a native driver's transition-to-draining -> defer: the real import fixture covers an admitted session and the existing N4 native barrier fixture covers the `Draining` refusal; no end-to-end importer-during-transition fixture was run
- [~] 4.5 @runtime (agent) force a local-close error and prove exact-owner quiescence keeps maintenance until that owner is reaped -> defer: the close-error branch binds quiescence to the captured checked-directory identity and was source-reviewed, but no forced close-error fixture was run
