# Verification

## 1. Restore commit and branch boundary [critical]

- [x] 1.1 @regression (agent) existing whole-product branch scanner and negative raw rename/delete shapes -> original Ubuntu1/ARM memory2 scanner failed on staged `-m`; owning memory task passed both unchanged scanner cases after the established message alias correction, no scanner edits
- [x] 1.2 @integration (agent) existing remapped_restore_preserves_dirty_main_usage_snapshots_and_fresh_authority -> actual owning native fixture passed with all distinct STAGED/WORKING, main/usage and fresh-authority assertions unchanged; memory selection total 3/3, zero ignored, 7.12s

## 2. Canonical project preferences [critical]

- [x] 2.1 @regression (agent) existing preferences_survive_reopening_without_resuming_chats_or_crossing_project_boundaries plus existing closing guard -> original Ubuntu6 foreign view unwrap failed; affected-only corrected native preferences case passed 1/1, zero ignored, 4.69s; existing closing guard passed in prior selected run; all original preferences/reopen/fresh-session/override/resume/final cleanup assertions retained

## 3. Static acceptance

- [x] 3.1 @equivalence (agent) affected memory/runtime host and Windows lint, typecheck, docs, format and managed checks -> all nine initial scoped checks exited 0; final affected runtime host/Windows lint, typecheck and format reruns also exited 0 after the intermediate direct-close adjustment; no authority, protocol, schema, dependency, workflow or budget changes
- [~] 3.2 @e2e (agent) fresh final-head full supported-platform CI and exact-main checks -> defer: native Linux/Windows execution and instrumented 90% acceptance require the corrected archived public head; mandatory remote delivery remains pending

## Original evidence

Published `1ad442f7` failed the unchanged branch scanner on Ubuntu1 and ARM
memory2, identifying `store/backup_restore.rs:181:11` and its staged message flag.
Ubuntu6 failed the preferences fixture at line 75 with `memory view belongs to a
different canonical project`. These official failures are retained; no unchanged
retry or speculative production ownership change is proposed.

## Local correction evidence

The first local runtime selection passed its closing guard but failed at the new
intermediate `close_stores([other_memory])` call: that helper requires every
supervisor closed while the original view intentionally remained live. Replace
that intermediate call with the same new store's explicit awaited `close()`,
retaining the final universal `close_stores([memory])` proof. The affected-only
rerun passed; the intermediate failure is not hidden or counted as a passing run.
Initial strict/apply artifact validation required the integration contract section
and a known ledger owner; both were corrected and all five returned contexts read
before any source implementation. No additional production authority or scanner
exception was introduced.
