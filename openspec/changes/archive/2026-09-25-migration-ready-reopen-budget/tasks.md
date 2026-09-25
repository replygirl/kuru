## 1. Retained-ready recovery fixture

- [x] 1.1 Update `packages/kuru-memory/src/store/recovery_tests.rs` so the final writable reopen uses the existing configured migration observation budget and reports distinct outer-deadline and inner-open errors; verify all retained-ready assertions remain.
- [x] 1.2 Run the exact real-Dolt recovery test and required formatting/Cospec checks; record the result and leave native Intel CI as an explicit premerge gate.

Observed on the corrected source: the exact real-Dolt `absent_fast_forward_keeps_the_same_ready_attempt_for_next_open` filter passed 1/1 in 4.63 seconds after package-owned prefetch, and Rust format passed. The previous native Intel job failed at the unlabelled deadline; corrected-head native Intel CI and the full combined coverage gate remain required before merge.
