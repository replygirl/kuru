## 1. Held candidate fixture

- [x] 1.1 Update `packages/kuru-memory/src/store/operational_gc_tests.rs` to observe the held connection ID and await its actual session teardown after drop, preserving the first refusal and final candidate/history assertions.
- [x] 1.2 Run the focused real-Dolt fixture and scoped Rust checks, record platform limits, and validate strictly.

## Evidence

- macOS real-Dolt fixture: `KURU_DOLT_BUNDLE_DIR=/private/tmp/kuru-phase1-bundles mise run //packages/kuru-memory:test -- held_candidate_view_delays_explicit_abandonment_without_losing_history` passed 1/1.
- Scoped Rust lint: `KURU_DOLT_BUNDLE_DIR=/private/tmp/kuru-phase1-bundles mise run //packages/kuru-memory:lint` passed. `mise run //:format:rust` and `git diff --check` passed.
- Native Windows recurrence remains unrun for this detached commit; the prior P9 head's Dolt 1105 in-use refusal after dropping the held connection motivated this exact-session wait.
