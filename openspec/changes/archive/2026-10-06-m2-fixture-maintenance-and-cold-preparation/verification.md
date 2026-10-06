# Verification

## 1. Correct fixture observation and cold preparation [critical]

- [x] 1.1 @regression (agent) run managed_driver_presence_fences_switches_and_unproved_reattachment -> owning task54083 exit0, named case passed; typed native contention while hold remains, unchanged directory/no-intent/released-hold purge/reap assertions passed
- [x] 1.2 @regression (agent) run a_new_project_imports_empty_scope_without_losing_other_projects -> task54083 exit0, named case passed; original source/history/new turn/close assertions preserved on existing cold legacy creation path
- [x] 1.3 @regression (agent) run wal_import_preserves_original_other_projects_order_and_opaque_json -> task54083 exit0, named case passed; ordered Unicode/NUL content, opaque JSON, other-project isolation, original database/WAL bytes and same-revision reopen assertions passed
- [x] 1.4 @unit (agent) run driver_async_fixtures_use_closing_scope -> task54083 exit0, existing guard passed; total selected4/4 in5.70s, owning task134.29s including preparation/compilation, remaining targets0 selected

## 2. Scoped checks and evidence limits

- [x] 2.1 @integration (agent) run memory host and Windows lint and all-target typecheck -> task34690 exit0; host lint52.70s, Windows target lint40.12s, all-target/all-feature typecheck9.57s; no native Windows execution claimed
- [x] 2.2 @integration (agent) run docs, format, managed drift and final strict/apply context review -> docs/content and managed task80333 completed0; final format6095 exit0; final strict/apply22efab both0/all four returned contexts read
- [~] 2.3 @runtime (agent) run corrected native ARM fixture cases -> defer: local macOS checks do not establish ARM execution; Product owns fresh full PR CI

## Before-fix evidence

Official PR249 94ad967e job112520785435 failed both original cases. The driver
assertion expected removed text; its original refused error value was not printed
and remains unknown. The empty-scope case hit the fresh-store unwarmed guard at
migration.rs1537 before migration could run. These executed failures are the
regression baseline; no duplicate baseline execution is claimed.

Official ARM job112520785562 and Ubuntu coverage job112520787463 also fail the
WAL fixture at its unwarmed fresh open. Both migration fixtures had their warm-up
removed in the 5f600c47 to 94ad967e diff. Their prepared legacy imports select
cold creation before template selection in store/creation_worker.rs.

## Initial local attempts

Owning task59539 exited1 after preparation because the command omitted the
Cargo-to-libtest separator; no test ran. Static task24025 exited101 because it
omitted mise's multi-task separators. Corrected owning task48567 and static
task80333 reached compilation but failed E0616 at the attempted private cold
creation field assignment; no regression test ran. Docs build/content,
formatting and managed checks completed in80333, but no host/Windows/type pass
is claimed. The approved correction restores warmed-only preparation through
the existing cold legacy selector, without field visibility or helper changes.

## Final local execution

The single successful owning memory selection used the three full names and
existing driver closing guard with forwarded `-- --exact`, `ulimit -n 4096`,
`MBX_TARGET_VIEWS=0`, and the verified
`KURU_DOLT_CACHE=/private/tmp/kuru-dolt-test-cache-phase2.PXfSjA`. Package-owned
bundle/prepared-supervisor prerequisites completed. Source remained frozen during
the run. Root reviewed the exact final three fixture hunks and found no scope or
assertion change. All local native and compiler handles are terminal.

Final format task6095 exited0. Docs build/content and managed drift completed
successfully in task80333 before the independent compiler visibility failure;
no product docs or managed source changed afterward. Historical ARM refusal
type remains unknown; only the corrected local case positively observed the
typed contention error. Fresh corrected remote CI remains pending.
