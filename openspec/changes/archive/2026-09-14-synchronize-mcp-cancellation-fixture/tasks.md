## 1. Synchronize the cancelled peer

- [x] 1.1 In `packages/kuru-connectors/src/mcp.rs`, wait boundedly for the first empty transcript before aborting the pending catalog; verify that the original terminal count assertion then reproduces the failure.
  - Observed locally on macOS 2026-09-14: after the new 5-second, 10-millisecond-poll synchronization and before changing the old terminal assertion, `mise run //packages/kuru-connectors:test -- cancelled_pending_startup_is_cleaned_before_explicit_recovery_spawns` failed deterministically with `left: 2`, `right: 1`.
- [x] 1.2 Require exactly two transcripts with lengths zero and three, retaining cancellation, recovery availability and successful shutdown assertions; verify the corrected exact test and adjacent startup cleanup tests.
  - Observed locally on macOS 2026-09-14: the corrected exact test passed, as did `shutdown_waits_for_admitted_startup_cleanup_before_success` and `post_spawn_setup_failure_retains_owner_until_cleanup_is_confirmed`, each through `mise run //packages/kuru-connectors:test -- <exact test name>`.

## 2. Verify and deliver

- [x] 2.1 Run connector typecheck and formatting checks; record observed results and native CI checks still pending.
  - Observed locally on macOS 2026-09-14: `mise run //packages/kuru-connectors:typecheck` passed. An initial `mise run //:format:code` did not complete because Taplo panicked in `system-configuration` while checking TOML, and the first shared Rust-format check found a concurrent `tools.rs` line outside this MCP-only cut. After that source was formatted, root ran the full `mise run //:format:code` successfully with no edits. Native CI remains pending.
- [x] 2.2 Independently review the test-only diff, validate the change strictly and archive before the final commit.
  - Checkpoint's independent combined review was clear on 2026-09-14. Migration owns strict validation, archive, and the final commit; native CI remains pending.
