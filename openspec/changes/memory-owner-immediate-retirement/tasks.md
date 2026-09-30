# Tasks

Work packages own disjoint files (design D13 and the revised unit 1 design, section 11). All work runs in the worktree `tmp/worktrees/fix-memory-service-retire-when-unused`; `rpc.rs` edits stay inside `Retirement`, `serve_attached` and the handshake.

## 1. WP0 specs and early measurement

- [x] 1.1 Author the four MODIFIED deltas and run `mise run cospec -- validate memory-owner-immediate-retirement --strict`, and verify it reports no spec error (the misplaced four scenarios stay under the candidate requirement by name; see design D13). Observed 2026-09-29: the first run confirmed the four scenarios parse under "Reachable exact-ref candidate resolution" (living 11) and refused moving them (`archive/scenario-preservation`); after keeping them in place, validate passed with 0 errors, 0 warnings.
- [ ] 1.2 Measure V1 close duration and V2 reopen time on macOS and Ubuntu before the bulk of implementation (a first cut of the serve loop without the idle wait is enough), and verify the figures are recorded in verification 0.1 and 0.2; if a reopen right after a close typically waits more than about one second in total, stop before merge and report
- [ ] 1.3 Handle the `project-memory-service` `## Purpose` line ("retains idle service authority safely"), which no delta covers, and verify after archive whether it was rewritten; if not, record it for an explicit follow-up edit

## 2. WP1 Windows pipe marker (`packages/kuru-platform`)

- [ ] 2.1 Add `NoFreeInstance` and `pub fn is_no_free_instance` to `src/windows/pipe.rs`, keeping kind `TimedOut` and the exact message, and verify T14's platform half compiles under `mise run lint` (Windows target) and passes in native Windows CI

## 3. WP2 owner lifetime (`service.rs`, `service/rpc.rs`, `store.rs`)

- [ ] 3.1 Write the regression tests T1, T2 and T6m first and verify they fail on the unfixed tree for the stated reason (record the failure)
- [ ] 3.2 Add `OpenOptions::starter_token`, the optional tenth service argument and its parsing, the optional hello field, and `Retirement.reached` set in `serve_attached`; verify T15
- [ ] 3.3 Replace `serve_until_idle_with` by `serve_until_retired` with the two empty states, the absolute first-attachment deadline, `OWNER_LOCK_RECHECK_INTERVAL`, and delete `SERVICE_IDLE_TIMEOUT`; add the serve-loop observer; verify T1, T2, T3, T4, T5, T18
- [ ] 3.4 Reorder shutdown (listener, endpoint, store close and reap, explicit Owner unlock), add `ClosePause` and `ServiceLock::release`, and verify T7, T13 and the renamed reap-order test
- [ ] 3.5 Map Windows code 233 in `is_peer_closed`, consume the pipe marker in `try_attach_observed` and `request_idle_retirement`, map a peer-closed maintenance handshake to `None`, apply D9 to `attach_existing`, reword the endpoint error, and verify T6, T6m, T12, T14's service half
- [ ] 3.6 Add `held` and `retain_after_abandon` to `ServiceAttachment` (every struct literal), make `close()` clear both, add the drop guard's spawn point in `call_with_id`, add the compile-time `const` margin assertion with its comment, and add `DispatchPause`; verify the held-stream unit test and verification 4.7
- [ ] 3.7 Rewrite the service tests in the revised design's section 7.2 and audit by content every test that reads endpoint absence as "reaped" (including what follows the polling in the Windows starter-Job test); verify verification 1.4, 6.2 and 6.3
- [ ] 3.8 Correct the comments listed in the revised design's section 7.5 for these files

## 4. WP3 client side (`facade.rs`)

- [ ] 4.1 Add the session's primary `Weak`, the replacement guard and task, flag inheritance on the lazy path, the read-only rule and its error context, and the replacement test hook; verify T8, T8b, T8c, T8d, T9, T10
- [ ] 4.2 Add `RemoteSession.successor` and use it in every successor branch and in `reopen_after_checked_recovery`; analyse and test its effect on `active == 1` and `MAX_ATTACHMENTS`; verify T11 and T11a
- [ ] 4.3 Audit facade fixtures that relied on idle expiry and retire them explicitly; verify the kuru-memory suite passes

## 5. WP4 test support (`test_support.rs`, `served_owner.rs`, `spawn_gate.rs`)

- [ ] 5.1 Add `ServeKnobs`, `serve_with`, the never-reached default for `serve`, and `await_owner_release`; verify every in-process fixture still ends by maintenance, restart or lock loss
- [ ] 5.2 Add the test-support environment hook, forwarded like `lifecycle_trace::forwarded()`, that routes owner diagnostic stderr to whichever owner a CLI child elects; verify it is inert outside test support
- [ ] 5.3 Correct the comments in these files (revised design 7.5)

## 6. WP5 consumer tests (`apps/kuru-tui/tests`, `packages/kuru-runtime/src/dream.rs`)

- [ ] 6.1 Rewrite the sequential-commands CLI test (T16) and the logged-owner CLI test (T17) on the hook from 5.2, relabel the warm/cold print as "reopen", reword the trust-route comment and assert the route; verify `mise run //apps/kuru-tui:test`
- [ ] 6.2 Correct the comments in `tests/support/memory.rs` and `dream.rs`; verify `mise run //packages/kuru-runtime:test`

## 7. WP6 documentation

- [ ] 7.1 Update `docs/memory.md`, `docs/install.md`, `apps/kuru-docs/guide/installation.md`, `docs/configuration.md` and `docs/development.md` per the revised design section 10, including Option B's uncovered cases and the fixture hooks; verify `mise run docs:check`
- [ ] 7.2 Add the mixed-version upgrade note to `docs/release.md` if it has upgrade notes; verify the text matches design Risks

## 8. Verification and close

- [ ] 8.1 Run `mise run lint` (host and Windows target), `format:check`, `typecheck`, the package tests and cospec validate, and record each result in verification with its command; name unrun checks and why
- [ ] 8.2 Record V1-V4 from native CI legs; state that Windows behaviour is unverified until those legs pass
- [ ] 8.3 Complete tasks, validate, and run `mise run cospec -- archive memory-owner-immediate-retirement` before the final branch commit; confirm the archive exists
