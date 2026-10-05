# Tasks

## 1. Platform settlement

- [x] 1.1 Add absolute-deadline snapshot admission and owned helper cleanup; verify lock contention, delayed admission and helper deadline behavior.
- [x] 1.2 Add shared nonblocking pre-reap step and retained worker with fresh repeated-signal authority; verify phase/error, cancellation/resume and zombie/listing contracts.
- [x] 1.3 Add real deterministic second-sweep regression and negative control plus finite native fork supplement; verify cleanup restores real retained-root ownership before assertions.
- [x] 1.4 Retain one completion receiver on the existing worker and add a bounded platform wait; verify wake/cancellation/deadline/panic/unfinished-join controls without changing result authority.

## 2. Caller integration

- [x] 2.1 Drive the step from hook, RPC, shell and coverage loops using existing deadlines; verify caller errors, cancellation, output retention and zero wait.
- [x] 2.2 Update two public cleanup pages and platform module docs; verify documentation build and content checks.
- [x] 2.3 Exact-reap the exited coverage root only at final disposal; verify normal/interrupted pending-helper expiry remains unconfirmed, cancellation/resume retains ownership, and zero wait preserves its deadline.

## 3. Acceptance and archival

- [x] 3.1 Remeasure before/after normal hook/shell cleanup latency and concurrent batch snapshot cost after completion wakes; record median, p95 and helper count, remove temporary probes, and record the accepted correctness/latency tradeoff.
- [x] 3.2 Run scoped static checks and native macOS caller/runtime acceptance, recording hosted Linux checks honestly as unrun where unavailable.
- [x] 3.3 Integrate exact accepted main, verify reviewed source and commit equivalence, and run the scoped static/documentation checks justified by that integration.
- [x] 3.4 Complete the evidence ledger and actual strict/apply gates for archive readiness, retaining required fresh native PR CI before merge as explicitly unrun.

Actual closeout: normal Cospec archive exited 0 and confirmed
`openspec/changes/archive/2026-10-05-unix-retained-root-resignal/`, active-directory
removal and one modified native-platform requirement applied and verified.
Post-archive strict living-spec validation and managed checks passed. The final
branch commit through normal hooks remains mandatory before publication handoff;
its actual result is reported in that handoff rather than claimed in advance.
Root owns publication, required fresh native/instrumented PR CI and merge.
Hosted acceptance must pass before merge; the actual archive gate accepted the
explicit local-host deferral without force or skipped validation.
