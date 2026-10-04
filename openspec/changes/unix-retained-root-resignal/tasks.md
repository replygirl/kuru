# Tasks

## 1. Platform settlement

- [x] 1.1 Add absolute-deadline snapshot admission and owned helper cleanup; verify lock contention, delayed admission and helper deadline behavior.
- [x] 1.2 Add shared nonblocking pre-reap step and retained worker with fresh repeated-signal authority; verify phase/error, cancellation/resume and zombie/listing contracts.
- [x] 1.3 Add real deterministic second-sweep regression and negative control plus finite native fork supplement; verify cleanup restores real retained-root ownership before assertions.

## 2. Caller integration

- [ ] 2.1 Drive the step from hook, RPC, shell and coverage loops using existing deadlines; verify caller errors, cancellation, output retention and zero wait.
- [x] 2.2 Update two public cleanup pages and platform module docs; verify documentation build and content checks.

## 3. Acceptance and archival

- [x] 3.1 Measure before/after normal hook/shell cleanup latency and concurrent batch snapshot cost; record median and p95.
- [ ] 3.2 Run scoped static checks and native macOS caller/runtime acceptance, recording hosted Linux checks honestly as unrun where unavailable.
- [ ] 3.3 Complete the evidence ledger, strictly validate and archive the change before the final branch commit through normal hooks.
