## 1. Stale rewrite record

- [x] 1.1 Write a `cleared` tombstone before dispatch when a re-admitted turn is not rewritten, treat a matching tombstone as the original user entry, and verify with a regression test that fails before the fix (rewrite, stop before dispatch, retry allowed, later turn projects the original)
- [x] 1.2 Name the state key, turn ID and node ID in the projection's fail-closed errors, and verify that the crate compiles and clippy is clean

## 2. Flake and wording

- [x] 2.1 Release the first parallel call's post hook only after the runtime's `ToolSettled` event for the second call, and verify with at least 30 isolated passing iterations
- [x] 2.2 State the session-export and memory-export semantics and the retry rule in the docs and spec, correct the archived flake attribution, note the batched-lookup follow-up, and verify with `docs:check` and strict validation
- [x] 2.3 Run focused checks and record evidence and unrun checks in verification.md
