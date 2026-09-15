## 1. Pre-authentication reset recovery [critical]

- [x] 1.1 @regression (agent) use a direct portable `live_endpoint` test with an owned bounded listener that observes the raw published-endpoint probe and abortively resets the following SQL connection before `after_connect` -> `mise run //packages/kuru-memory:test -- pre_callback_connection_reset_is_not_a_live_endpoint` failed against the released policy with raw-probe success, `authenticate published memory endpoint`, `connection phase: after_connect not entered` and `Connection reset by peer (os error 54)`; the corrected exact test passed 1/1
- [ ] 1.2 @integration (agent) after proving the old Windows Job has zero active processes, reopen the real committed store through the same reset-listener shape and join the listener on every path -> the replacement owned server exposes the same committed row, completes ordinary shutdown and descendant cleanup, and the fixture proves both expected connections without detached work

## 2. Error and ownership boundaries

- [ ] 2.1 @regression (agent) run the existing wrong-credential, checked-directory and SQL-identity refusal controls -> each callback/authentication failure remains terminal with its original diagnostic and cannot be classified as a stale endpoint
- [ ] 2.2 @integration (agent) retain the existing occupied-lifecycle contender control during whole-Job loss recovery -> the stale endpoint result cannot bypass authoritative lifecycle ownership

## 3. Repository and native acceptance

- [x] 3.1 @unit (agent) run focused server and Windows lifecycle checks supported by the host, memory typecheck, lint, formatting, strict Cospec validation and diff checks -> the portable reset regression passed 1/1; memory typecheck and lint, Rust format fix/check, strict Cospec validation and `git diff --check` passed; the cfg(windows) whole-Job regression remains pending native CI
- [ ] 3.2 @runtime (agent) run the deterministic reset recovery and refusal controls plus ordinary Windows memory/runtime, installation and native-mise acceptance in PR CI -> all required native jobs and the fail-closed aggregate pass at one exact head before archive
