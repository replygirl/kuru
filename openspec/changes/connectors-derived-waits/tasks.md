# Tasks

Every fix point below is recorded as done (with its shape and bound source) or not
done (with a code-backed reason) when its task is ticked. A shape written here is the
intended one, confirmed against the code at scoping; the implementer confirms it
again and records what was done. Rules for every task: each bound is event-driven and
derived from a stated product value, with the derivation at the site or in the
constant's doc comment; no literal without a derivation; no retry; no change to
`startup_timeout_secs`; no product code (the `Policy` section of `proposal.md` says
how private values are restated); no edit to `hooks.rs`, `rpc.rs` or `unix_shell.rs`.
The assertions after each wait stay as they are, except the elapsed-time asserts that
a gate makes redundant (c1#133, c1#91).

## 1. Base and coordination

- [x] 1.1 Re-baseline the 29 fix points on origin/main 449dca9e and verify each cited line still holds the cited code
  Evidence (2026-10-04, read-only, worktree `test-connectors-derived-waits`): origin/main is 449dca9e, one commit past the inventory's 327f817c (#206, `packages/kuru-runtime` and `openspec` only); `git diff --stat 327f817c HEAD -- packages/kuru-connectors` is empty (0 lines) and so is the diff for `AGENTS.md` and `docs/development.md`. Every cited line printed the cited code: `auth/tests.rs` 94, 172, 702, 748, 799, 872; `mcp.rs` 3560, 3571, 5400, 4416, 5211, 5138, 4866, 5302; `mcp_credentials.rs` 1538; `providers.rs` 1801, 1810, 2251, 2267, 2293, 2302, 2303, 2751, 2789, 3032, 3044; `subscription_tests.rs` 1118, 1124, 1191, 1225, 1272, 1281, 1313, 1336, 1385; `test_support.rs` 251; `tools.rs` 4785, 4852, 4803, 4865, 5087, 5096, 5156, 5163, 5180, 5196, 5212, 5270, 5285, 5767; `web_fetch.rs` 269, 341-422, 381, 438, 563, 729; `stdio_peer.rs` 142. Two corrections: the brief's "mcp.rs ~4464" for c1#129 is 4416 (the site list agrees), and c1#37's `timeout_ms: 3_000` lines are 4798 and 4862, not 4799 and 4863. No site moved and none is already fixed. Visibility was measured, not assumed: `CLEANUP_ALLOWANCE` (`unix_shell.rs:36`), rpc `CLEANUP`/`GRACE`/`STDERR_DRAIN` (`rpc.rs:50-52`), `CALLBACK_TIMEOUT` (`auth/login.rs:17`), `DIAGNOSTIC_TIMEOUT` (`providers/diagnostics.rs:10`) and `LOCK_DEADLINE` (`mcp_credentials.rs:25`, from `mcp.rs`) are private to modules that are not ancestors of the test modules that would need them; `crate::IO_TIMEOUT` is `pub(crate)` and `COMPLETION_TIMEOUT` and `STREAM_IDLE_TIMEOUT` are `pub`. None of the 29 needs an edit to a PR #207 file under the restate policy, so none is deferred for that reason. Re-check each line again at edit time.
- [ ] 1.2 Check coordination with PR #207 and PR #212 before opening the PR and verify `git diff --stat origin/main -- packages/kuru-connectors/src/hooks.rs packages/kuru-connectors/src/rpc.rs packages/kuru-connectors/src/unix_shell.rs` is empty on the branch head, and that #212 is told it waits for this PR

## 2. Stdio peer and shared support (`tests/fixtures/stdio_peer.rs`, `src/test_support.rs`)

- [ ] 2.1 c1#7: add `Step::Park` (a `park` operation that drains stdin without recording to the transcript until the product closes it) and a release-file gated step (the peer waits until a path the test creates exists), with an arm for each in `StdioFixture::with_cache`'s exhaustive `Step` match; keep `Step::Sleep` and every existing step's behaviour; replace `Step::Sleep(5_000)` at `mcp.rs:4866` and `mcp.rs:5302` with `Step::Park`; leave `rpc.rs:1000,1043`; verify by `mise run //packages/kuru-connectors:format:fixture:check`, by the two MCP tests passing with their transcripts unchanged (`conversations()[0].len() == 4`), and by reading the diff for no remaining `Step::Sleep` outside `rpc.rs`
- [ ] 2.2 c1#64: derive `wait_for_requests` from `crate::IO_TIMEOUT` (the `Rpc::request` timer, `rpc.rs:545,555,1068-1071`) with the derivation in a comment, keeping its panic text and `diagnostics()`; verify by the callers `rpc.rs:1078` and `mcp.rs:4897,5202,5321` still passing and by `rpc.rs` being absent from the diff

## 3. Auth (`src/auth/tests.rs`)

- [ ] 3.1 c1#111: derive `callback_request`'s read bound from the callback read, the token exchange and the reply write (`CALLBACK_TIMEOUT` restated once, `crate::IO_TIMEOUT` imported), passed or defined once for its 5 callers (257, 346, 385, 1204, 1297); verify by the native auth browser and callback tests (`native_auth_browser_exchanges_exact_pkce_and_publishes_private_session`, `native_auth_invalid_callbacks_and_cancellation_preserve_old_session`, `native_auth_browser_busy_port_expiry_and_inflight_cancel_are_owned`) passing and by the derivation naming `login.rs:17,52,235` and `http.rs:98`
- [ ] 3.2 c1#117: bound the two `gate.reached.notified()` waits (702, 748) by `manager.inner.http_timeout`, the refresh allowance; verify by the two tests passing and by no `from_secs(10)` left at those lines
- [ ] 3.3 c1#118: bound the post-release `&mut refresh` wait (799) by the same allowance; verify by `owned_refresh_gate_retries_once_after_safe_refusal_and_publishes` passing
- [ ] 3.4 c1#120: bound the `BeforeFirst` gate wait (872) by `manager.inner.http_timeout` in both iterations of `owner_gate_caller_loss_and_expired_grant_stop_before_first_post`; record the tie with the 2 s allowance in the `expire` iteration; verify by both iterations passing
- [ ] 3.5 c1#110: bound `Fixture::arrived` (94) by `self.manager.inner.http_timeout`; verify by its callers (393, 1051, 1093) passing

## 4. MCP (`src/mcp.rs` test module)

- [ ] 4.1 c1#127: bound 3560 by `LOCK_DEADLINE` (restated once from `mcp_credentials.rs:25`), 3571 by `IO_TIMEOUT` plus `LOCK_DEADLINE`, and 5400 by `crate::IO_TIMEOUT` as a named upper bound, each with its chain written at the site and 5400 recorded as an inference; verify by `oauth_issuer_change_and_logout_revoke_cached_tool_routes_until_live_discovery` and `cancelled_pending_startup_is_cleaned_before_explicit_recovery_spawns` passing
- [ ] 4.2 c1#129: derive the isolated child's bound (4416) from `initialize` and `tools/list` (2 x `IO_TIMEOUT`) plus the `McpHosts::shutdown` join (`IO_TIMEOUT`, `mcp.rs:1203`), with the peer's one rustc compile noted as outside every product budget; verify by `isolated_stdio_mcp_keeps_inherited_environment_and_configured_override` passing and by the panic text and cleanup drains unchanged (the failing run is #212's 37180709856)
- [ ] 4.3 c1#131: hold the slow alias's peer on the release-file gated step from 2.1 at `mcp.rs:5138` instead of `Step::Sleep(400)`, create the release file after `other_alias` returns, and bound that wait by `IO_TIMEOUT`; verify by `multipage_discovery_serializes_same_alias_while_other_alias_proceeds` passing and by `assert!(!same_alias.is_finished())` now holding because the slow peer is held, not because 400 ms had not elapsed

## 5. MCP credentials (`src/mcp_credentials.rs` test module)

- [ ] 5.1 c1#8: replace `sleep(75 ms)` plus `!is_finished()` (1538) by polling the pinned `acquire("first")` future once while `held` is alive (`futures::poll!`; `futures` is already a dependency), asserting it is pending, then dropping `held` and awaiting the same future; verify by `lock_is_project_alias_bound_and_serializes_without_secret_bytes` passing and by a temporary swap that lets the contender acquire early failing the pending assert (record, do not commit)

## 6. Providers (`src/providers.rs` test module)

- [ ] 6.1 c1#79: bound the two event waits (1801, 1810) by `COMPLETION_TIMEOUT` (stream timer `providers.rs:1112-1117` over connect and `STREAM_IDLE_TIMEOUT`); verify by `api_sse_delta_reaches_observer_before_delayed_terminal` passing
- [ ] 6.2 c1#85: make the server answer only after the generic client deadline is observed on the client side (a later probe request on the same generic client times out first), not after `sleep(150 ms)`; follow the outer 3 s (2272) to `provider.completion_timeout`; verify by `api_key_completion_overrides_shorter_generic_client_deadline` passing and by a temporary swap that drops the override failing it (record, do not commit)
- [ ] 6.3 c1#89: release the model list from a gate after the 80 ms completion deadline is observed (a `complete()` on the same provider times out); follow the `timeout(1 s, server)` at 2308; verify by `api_key_catalog_keeps_its_separate_http_deadline` passing
- [ ] 6.4 c1#90: bound `provider.models()` (2303) by `crate::IO_TIMEOUT` (`providers.rs:1061,1069`); verify by the same test
- [ ] 6.5 c1#133: hold the socket on a gate released after `complete()` returns and drop the `< 3 s` assert (record that a missing diagnostic bound is then reported at the completion deadline, and whether a derived elapsed check is kept); verify by `stalled_error_body_uses_status_fallback_inside_operation_budget` passing in about the 2 s diagnostic deadline
- [ ] 6.6 c1#91: dribble until a gate released after `complete()` returns, keep the 700 ms cadence below `DIAGNOSTIC_TIMEOUT` (restated in a comment), and drop the `< 2600 ms` assert; record that a per-chunk-restart regression now fails at the completion deadline instead of at 2.6 s; verify by `dribbling_error_body_uses_one_total_diagnostic_deadline` passing
- [ ] 6.7 c1#95: bound the request-count wait (3032) by `COMPLETION_TIMEOUT`; verify by `retry_exhaustion_and_backoff_cancellation_never_send_late` passing and record whether the `yield_now` spin is kept
- [ ] 6.8 c1#96: replace `sleep(1 s)` (3044) by the causal check (aborted task joined as cancelled, exact request count after the continuation, a replay failing `unexpected fixture request`); if a replay is not shown to fail without the sleep, record it as not done (absence) with the reason; verify by `retry_exhaustion_and_backoff_cancellation_never_send_late` passing

## 7. Subscription tests (`src/providers/subscription_tests.rs`)

- [ ] 7.1 c1#99: bound the five waits (1118, 1124, 1313, 1336, 1385) by `provider.completion_timeout`; verify by `cancelled_partial_stream_closes_socket_and_releases_same_actor`, `fragmented_discarded_subscription_sse_does_not_exhaust_retained_output_budget`, `missing_content_type_accepts_fragmented_subscription_sse` and `missing_content_type_still_rejects_malformed_and_truncated_streams` passing
- [ ] 7.2 c1#102: derive the fixture guard in `raw_subscription_stream_with_chunks` (1191) from `COMPLETION_TIMEOUT`, for its 3 callers; verify by the callers passing
- [ ] 7.3 c1#103: remove the timer around `released.await` in `raw_subscription_stream_paused_after_delta` (1225) and guard accept/read and each write phase separately under `COMPLETION_TIMEOUT`; verify by `subscription_sse_delta_reaches_observer_before_delayed_terminal` passing
- [ ] 7.4 c1#104: bound the two delta waits (1272, 1281) by `COMPLETION_TIMEOUT`, as 6.1; verify by the same test

## 8. Tools (`src/tools.rs` test module)

- [ ] 8.1 c1#37: replace `exec sleep 5` (4785, 4852) by a command that blocks on an unwritten FIFO, keep `timeout_ms: 3_000` (4798, 4862) and the secret and command-redaction asserts; verify by `unix_shell_timeout_projects_a_fixed_failure_without_captured_stderr` and `unix_shell_timeout_keeps_eof_complete_redacted_stderr` passing
- [ ] 8.2 c1#44: derive the shutdown backstops from `CLEANUP_ALLOWANCE` (restated once: 4803, 4865, 5096, 5285) plus a written allowance, and from `IO_TIMEOUT` for the shell and MCP shutdown at 5212; verify by the five tests passing and by the derivation naming `unix_shell.rs:36,577-600` and `mcp.rs:1203`
- [ ] 8.3 c1#40: restate the 120_000 ms shell maximum once as a const used for both the `timeout_ms` passes (5082, 5175, 5265) and the readiness waits (5087, 5180, 5270); verify by the three tests passing
- [ ] 8.4 c1#41: bound the three waits (5156, 5163, 5196) by the same const, noting that 5196 has no budget of its own; verify by `unix_shutdown_cancels_starting_and_active_shells_and_closes_mcp` passing
- [ ] 8.5 c1#53: derive the isolated child's bound (5767) from the shell default timeout (30 s, restated from `tools.rs:1344`) plus `CLEANUP_ALLOWANCE`; verify by `isolated_toolhost_shell_receives_only_compatibility_environment` passing

## 9. Web fetch (`src/web_fetch.rs` test module)

- [ ] 9.1 c1#10: derive `FIXTURE_TIMEOUT` (269) from `FETCH_TIMEOUT` (`web_fetch.rs:24`) with the derivation at the constant, and the `+ 1 s` addends (381, 438) from the longest fixture `delay` (200 ms, `:699,:721`); its dependents (341-422, 563, 729) follow; verify by the web fetch tests (including `independent_local_fetches_overlap_with_separate_checked_resolvers`) passing and by no remaining `FIXTURE_TIMEOUT` use without that derivation

## 10. Checks

None of these has been run at scoping; each needs observed results recorded when it runs.

- [ ] 10.1 Run `mise run format:check`, `mise run //packages/kuru-connectors:format:fixture:check` and `mise run typecheck` and record the exit codes
- [ ] 10.2 Run `mise run //packages/kuru-connectors:lint` and `mise run //packages/kuru-connectors:lint:windows` and record the exit codes
- [ ] 10.3 Run `mise run //packages/kuru-connectors:test` and record the pass count; state that a local run does not reproduce the loaded CI runner, and compare the isolated MCP child test (c1#129) over repeated runs
- [ ] 10.4 Validate with `mise run cospec -- validate connectors-derived-waits --strict`, confirm the apply gate is clear, and archive before the final commit
