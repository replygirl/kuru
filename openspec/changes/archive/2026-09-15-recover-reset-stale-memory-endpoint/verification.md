## 1. Pre-authentication reset recovery [critical]

- [x] 1.1 @regression (agent) use a direct portable `live_endpoint` test with an owned bounded listener that observes the raw published-endpoint probe and abortively resets the following SQL connection before `after_connect` -> `mise run //packages/kuru-memory:test -- pre_callback_connection_reset_is_not_a_live_endpoint` failed against the released policy with raw-probe success, `authenticate published memory endpoint`, `connection phase: after_connect not entered` and `Connection reset by peer (os error 54)`; the corrected exact test passed 1/1
- [x] 1.2 @integration (agent) after proving the old Windows Job has zero active processes, reopen the real committed store through the same reset-listener shape and join the listener on every path -> Windows memory/runtime job `104288586845` passed `enclosing_job_loss_contains_the_tree_and_reopens_committed_state` in the 12/12 lifecycle target, including both listener observations, committed-row continuity, cleanup and ownership assertions

## 2. Error and ownership boundaries

- [x] 2.1 @regression (agent) run the existing wrong-credential, checked-directory and SQL-identity refusal controls -> Windows memory/runtime job `104288586845` passed `wrong_credentials_directory_and_sql_identity_fail_without_server_takeover` with the complete 102/102 memory library suite
- [x] 2.2 @integration (agent) retain the existing occupied-lifecycle contender control during whole-Job loss recovery -> Windows memory/runtime job `104288586845` passed the whole-Job lifecycle test with its authoritative lifecycle-lock contender assertion

## 3. Repository and native acceptance

- [x] 3.1 @unit (agent) run focused server and Windows lifecycle checks supported by the host, memory typecheck, lint, formatting, strict Cospec validation and diff checks -> the portable reset regression passed 1/1; memory typecheck and lint, Rust format fix/check, strict Cospec validation and `git diff --check` passed; the cfg(windows) whole-Job regression remains pending native CI
- [x] 3.2 @runtime (agent) run the deterministic reset recovery and refusal controls plus ordinary Windows memory/runtime, installation and native-mise acceptance in PR CI -> exact-head run `34940694839` passed all static and native jobs: memory/runtime `104288586845`, application/native-mise `104288586942`, installation `104288587053`, four-receipt coverage report `104294078572`, native aggregate `104296240808` and the final CI gate
