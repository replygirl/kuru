## 1. Owned checkpoint lease ends at Drop [critical]

- [x] 1.1 @regression (agent) duplicate the held File from a real `CheckpointStore` lease, prove a fresh lease is excluded while the owner is active, then drop the owner and try a fresh lease while the duplicate remains open -> exact macOS filter session 76001 failed before the fix with post-Drop `file checkpoint store is busy`/WouldBlock (0/1), then session 44240 passed after Unix Drop unlock (1/1); the active-owner WouldBlock assertion stayed in place and neither run retried or waited for the lock.
- [x] 1.2 @integration (agent) run the existing actor file-edit exact-receipt/selected-undo and checkpoint inventory cases against real private files -> macOS owning `file_edits::tests::` passed 9/9 with the package's `RUST_TEST_THREADS=2` policy, and the original `actor_file_edit_reuses_exact_receipt_and_selected_undo_restores_source` passed 1/1; exact receipt, active-owner refusal and selected undo remained intact.

## 2. Native and repository gates

- [x] 2.1 @regression (agent) run owning connector all-target typecheck/lint, format and strict Cospec/apply -> owning connector all-target Clippy and exact-file rustfmt check passed; strict Cospec validation reported 0 errors/0 warnings and apply exited 0 with a clear gate.
- [~] 2.2 @integration (agent) observe exact-head native macOS connector CI plus Windows/Linux affected shard results -> defer: supported-platform CI requires the archived fix to be committed and pushed; the P11 publisher owns that final-head gate, and this local proof does not establish the exact historical CI spawn timing.
- [~] 2.3 @regression (agent) run normal branch hooks and record final-head combined coverage -> defer: hooks run during the publisher's normal push after this required archive and separate fix commit; no hook or coverage result is claimed here.

The first owning-module invocation omitted the package's two-thread test policy
and failed 2/9 with `Too many open files`; the same source under the owning
policy passed 9/9. The first exact regression compile failed on an unqualified
test-only macro; after that syntax correction the unmodified production code
failed behaviorally as required (0/1), and the same filter passed after the
minimal Unix Drop change (1/1). Neither resource nor syntax failure is counted
as the pre-fix behavioral red result.
