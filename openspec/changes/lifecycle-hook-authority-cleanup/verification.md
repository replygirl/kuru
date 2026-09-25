## Evidence environment

Local evidence was gathered on macOS arm64 on 2026-09-25 in the #89 worktree.

**Private target directory.** The shared `CARGO_TARGET_DIR` was being overwritten by concurrent builds of other worktrees. Those builds produce identical unit hashes, and one of them served a stale `kuru-core` and a stale test binary to this branch. The shared-target results were therefore discarded. Every result below comes from a private target directory, used with the same package test environment: `RUST_TEST_THREADS=2`, `KURU_TEST_SUPERVISOR_PREPARED=1`, and a `kuru-memory prefetch` run into that target.

**Negative checks.** Each regression was confirmed by temporarily disabling only the fix it covers, rerunning the test, and then restoring the source. Restoration was verified by grep.

## 1. Pre-tool authority [critical]

- [x] 1.1 @regression (agent) deliberation pre-tool hook rewrites `remember` to an allowed `a2a_send` (`hook_tests::deliberation_hook_cannot_turn_a_cognitive_call_into_external_dispatch`) -> passed with zero A2A sends; with the connector name check and runtime offered-set check disabled it failed with 2 sends (`left: 2, right: 0`)
- [x] 1.2 @unit (agent) connector chain receives a name-changing rewrite (`invalid_intermediate_rewrite_never_reaches_a_later_hook`, new case) -> passed and the later hook never launched; failed with the name check disabled
- [x] 1.3 @integration (agent) final-argument checks (`rewritten_file_read_is_checked_against_the_final_root_before_execution`, `granted_file_read_cannot_be_rewritten_into_shell_or_mcp`, `permission_tests` 12/12) -> all passed; substitution into shell or MCP yields a typed pre_tool `failed` observation, no successful settlement, no shell marker and zero MCP calls
- [x] 1.4 @integration (agent) cross-platform substitution fixture (`hook_platform_tests::pre_tool_tool_substitution_is_refused_before_dispatch`) on macOS -> passed

`deliberation_a2a_is_refused_as_an_unoffered_tool_without_dispatch` replaces the former approval-path expectation (`Denied`) with an `Error` settlement and zero sends, because deliberation never offers `a2a_send`. Speaking-phase typed permission refusals are unchanged: the ToolHost still evaluates the exact final non-cognitive call.

## 2. Owned cleanup [critical]

- [x] 2.1 @regression (agent) Unix cancellation after an observed readiness marker (`hooks::tests::cancellation_awaits_reaping_the_started_owned_hook_tree`) -> passed; the group was present before cancellation and `killpg(group, 0)` reported it absent after `quiesce`
- [x] 2.2 @regression (agent) runtime pre-turn cancellation (`hook_platform_tests::cancelled_pre_turn_hook_is_reaped_before_the_turn_returns`) -> passed with zero in-flight hooks at turn return and no provider request; with the turn-level `await_hook_cleanup` removed it failed 3/3 with one hook still in flight
- [x] 2.3 @integration (agent) cancelled dream (`cancelled_dream_abandons_candidate_hook_annotations_and_reaps_hook_descendants`, now asserting zero in-flight hooks at return) -> passed
- [x] 2.4 @regression (agent) Unix descendant escaped with `setsid` holds stdout (`escaped_descendant_holding_stdout_fails_the_hook_within_its_deadline`) -> passed in about 2 s with a failed outcome; with the post-exit drain unbounded it hung until the descendant self-released, returned an allowed outcome and failed after 70.88 s
- [~] 2.5 @integration (agent) Windows cancellation and runtime hook flows -> defer: native Windows CI on the pushed head; the Windows code has not been compiled because the local `x86_64-pc-windows-msvc` cross-check stopped in the `aws-lc-sys` C build, which lacks Windows SDK headers

The row 2.5 paths are:
- the `PSModulePath` launch policy
- the hook warm-up
- `windows_cancellation_awaits_reaping_the_started_hook`
- the Windows arms of `hook_platform_tests`

## 3. Records and diagnostics

- [x] 3.1 @integration (agent) runtime pre-turn rewrite (`inspection_skips_hooks_while_runtime_rewrite_preserves_the_durable_input_and_replay`, `hook_platform_tests::pre_turn_rewrite_reaches_the_provider_with_durable_hook_provenance`) -> passed; each rewritten current input in private history follows a `kuru-hook` pre_turn `rewritten` record, and neither private rows nor provider requests contain the original, which stays in the public history
- [x] 3.2 @unit (agent) suppressed host with an explicit origin value (`suppressed_hosts_report_every_configured_hook_without_running_it`) -> passed; one `suppressed` observation per configured hook, no command ran, no invocation was charged, the pre value was unchanged and speaker dispatch continued
- [x] 3.3 @unit (agent) Windows hook `PSModulePath` selection (`windows_hooks_keep_deliberate_module_paths_except_for_stock_powershell`) on macOS -> passed against the pure selection function; the native launch site remains row 2.5
- [x] 3.4 @integration (agent) typed assertions across the runtime hook suite -> passed; tests match `Event::Hook` and `Event::ToolSettled` fields instead of detail substrings

Row 3.4 also covers two further changes:
- The parallel post-hook ordering test uses a marker handshake instead of `sleep 0.2`.
- The CLI and engine share `HOOK_ANNOTATION_UNRESOLVED_AFTER_ANSWER` instead of restating the string.

## 4. Calibration

- [x] 4.1 @unit (agent) injected-clock budget (`budget_counts_overlap_once_and_leaves_idle_time_uncharged`, `budget_refuses_new_work_once_time_reaches_zero`) -> passed; two overlapping leases over 150 ms charged 150 ms, idle time was uncharged, exhaustion at zero refused new work, and the invocation cap refused the fourth claim
- [x] 4.2 @integration (agent) marker-synchronized real-process budget (`shared_budget_holds_leases_through_owned_reap_and_caps_invocations`) -> passed; two leases were active while release files held both hooks, zero after reap, and the fourth hook was refused without launch; the old wall-clock test was removed and no budget was raised
- [~] 4.3 @integration (agent) Windows hook test with the shared hook-launch warm-up and the 5 000 ms default -> defer: native Windows CI; the 120 s per-hook and 360 s aggregate literals are gone, and aggregates and observation waits derive from `timeout_ms`

## 5. Final head

- [~] 5.1 @runtime (agent) native macOS, Linux and Windows CI plus combined coverage of at least 90% -> defer: this task does not push; the lead serializes pushes and the pre-push hook runs full coverage, so no CI result is claimed
- [x] 5.2 @integration (agent) focused macOS suites, lint, format and docs -> all passed

Row 5.2 ran:
- `cargo test -p kuru-connectors --lib`: 295/295
- `cargo test -p kuru-runtime --lib -- hook_platform_tests hook_tests permission_tests`: 36/36
- `kuru` terminal hook PTY tests: 3/3
- CLI post-turn reporting: 1/1
- lifecycle-hook trust preflight: 1/1
- clippy on `kuru-connectors`, `kuru-runtime` and `kuru` with `-D warnings`: clean
- `cargo fmt --all -- --check`: clean
- `mise run docs:check`: passed

## 6. Agent behavior

- [~] 6.1 @eval (agent) live model behavior with offered-set admission and pre-turn provenance records -> defer: deterministic admission and records are covered by the fake-provider runtime matrix (rows 1.1 and 3.1); a live or paid provider evaluation needs user participation and is not run here
