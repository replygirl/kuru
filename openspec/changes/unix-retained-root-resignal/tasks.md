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

- [ ] 3.1 Remeasure before/after normal hook/shell cleanup latency and concurrent batch snapshot cost after completion wakes; record median, p95 and helper count, remove temporary probes, and obtain explicit cost disposition.
- [x] 3.2 Run scoped static checks and native macOS caller/runtime acceptance, recording hosted Linux checks honestly as unrun where unavailable.
- [ ] 3.3 Complete the evidence ledger, strictly validate and archive the change before the final branch commit through normal hooks.
