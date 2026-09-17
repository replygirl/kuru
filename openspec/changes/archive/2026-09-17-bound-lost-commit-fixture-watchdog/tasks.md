## 1. Migration recovery fixture

- [x] 1.1 In `packages/kuru-memory/src/store/recovery_tests.rs`, bound the lost-commit-reply opening await with the existing migration observation deadline and include the two proxy progress flags on timeout, retaining exact-once and session-end assertions.
- [x] 1.2 Run the focused memory fixture and scoped static checks, record platform limits, and validate strictly.

## Evidence

- macOS real-Dolt fixture: `KURU_DOLT_BUNDLE_DIR=/private/tmp/kuru-phase1-bundles mise run //packages/kuru-memory:test -- production_upgrade_reconciles_lost_commit_reply_after_routed_session_ends` passed 1/1.
- Scoped Rust lint: `KURU_DOLT_BUNDLE_DIR=/private/tmp/kuru-phase1-bundles mise run //packages/kuru-memory:lint` passed. `mise run //:format:rust` and `git diff --check` passed.
- Native Windows recurrence remains unrun for this detached commit; its 10-second failure on the previous P8 head is the reason for this fixture-only watchdog correction. Production deadlines and behavior remain unchanged.
