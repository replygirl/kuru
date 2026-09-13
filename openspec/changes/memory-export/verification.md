## 1. Revision-pinned complete storage snapshot [critical]

- [x] 1.1 @integration (agent) use real pinned Dolt to capture main, advance it, create candidate and dirty rows, and page past one boundary -> every emitted record/count/schema/revision is from the captured commit-qualified pool only (`mise run //packages/kuru-memory:test -- export_`, 3 passed)
- [x] 1.2 @integration (agent) store signed sequence boundaries, unknown namespaces, unknown state keys/fields, empty tables, and one large admitted row -> records retain exact storage identity/order/payload and malformed data fails rather than being skipped (`mise run //packages/kuru-memory:test -- export_`, 3 passed)
- [x] 1.3 @regression (agent) reuse a cursor with another snapshot and inject a later page/count failure -> no successful complete snapshot is produced (foreign cursor plus real malformed later state page; `mise run //packages/kuru-memory:test -- export_`, 3 passed)

## 2. Provider-free command and faithful output [critical]

- [x] 2.1 @e2e (agent) run `kuru memory export` against an existing isolated store without provider credentials, then a fresh store and legacy SQLite sentinel -> existing JSON/Markdown snapshots succeed while fresh inspection does not import or create memory (`memory_export_is_provider_free_and_publishes_one_committed_snapshot`, `fresh_inspection_never_provisions_memory_and_history_is_read_only`, and `notes_cli_reads_existing_modes_without_provider_or_legacy_import`)
- [x] 2.2 @integration (agent) compare JSON and Markdown rendered from one snapshot containing arbitrary backticks and unknown values -> both preserve the same provenance and selected records without structural injection (`memory_export_is_provider_free_and_publishes_one_committed_snapshot`)
- [~] 2.3 @integration (agent) use stdout, refused existing destination, uncertain checked publication, and failed serializer/page fixtures -> defer: observed stdout, existing-destination rejection, and malformed later-page failure with no output/stage; deterministic uncertain-publication injection remains unrun

## 3. Quality and supported platforms

- [x] 3.1 @integration (agent) run focused memory and TUI tests plus format, lint, typecheck, and documentation checks -> executed granular gates pass with prepared supervisors (`export_` 3 passed; three focused TUI cases passed; memory/TUI lint and typecheck, Rust format, docs check exit 0)
- [~] 3.2 @integration (agent) run native Windows memory export and checked publication fixture -> defer: native Windows runner is unavailable locally
- [~] 3.3 @integration (agent) run the coordinated workspace coverage writer -> defer: root schedules the single shared coverage writer
