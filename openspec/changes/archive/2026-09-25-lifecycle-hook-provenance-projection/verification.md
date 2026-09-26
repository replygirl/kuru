## Evidence environment

Local evidence comes from macOS arm64 on 2026-09-25, in the #89 worktree, with its own freshly cleaned `target/`. The previously shared target (72.5 GiB) was removed with `cargo clean` before any build. The package test environment was `RUST_TEST_THREADS=2` and `KURU_TEST_SUPERVISOR_PREPARED=1`, after `//packages/kuru-memory:prefetch`.

Focused runs:
- `cargo test -p kuru-connectors --lib hooks::`: 14/14 passed.
- `cargo test -p kuru-runtime --lib -- hook_platform_tests hook_tests permission_tests accounting_tests::compaction`: 37/37 passed.

Static checks:
- `cargo fmt --all --check` is clean.
- `cargo clippy -p kuru-connectors -p kuru-runtime -p kuru --all-targets --all-features -- -D warnings` is clean.
- `mise run docs:check` passed.

Deviation: the code for this fix was drafted before these artifacts were written. The apply gate was then run (`validate --strict` passed; `apply --json` returned the clear state with exit 0) before any test, evidence or commit.

## 1. Provenance record stays private [critical]

- [x] 1.1 @regression (agent) `hook_platform_tests::pre_turn_rewrite_reaches_the_provider_without_its_durable_hook_provenance` rewrites a turn, runs a follow-up turn and compacts the part -> passed. No request carried a `kuru-hook` record or the original input, `user: platform rewrite` reached the provider, and private history keeps the pre-turn record immediately before it. With only the ordinary-request filter disabled it failed at `hook_platform_tests.rs:176`, and `hook_tests.rs:383` failed as well. With only the compaction filter disabled it failed at `hook_platform_tests.rs:176`. Both filters were restored afterwards (grep count 2).
- [x] 1.2 @integration (agent) `hook_tests::inspection_skips_hooks_while_runtime_rewrite_preserves_the_durable_input_and_replay` -> passed; no request carries the provenance record, and the deliberation request ends with the rewritten input
- [x] 1.3 @integration (agent) post-hook annotation tests in `hook_tests` (`post_tool_annotation_stays_with_its_actor_and_can_be_omitted_by_context_fit` and the related annotation cases) -> passed; annotations are unaffected by the filter

## 2. Post-exit cleanup bound

- [x] 2.1 @integration (agent) `hooks::tests::cancellation_after_root_exit_stops_the_held_output_drain` -> passed. `quiesce` returned Ok with 0 in-flight workers while the escaped descendant was still alive. This is not a before/after regression: the defect needs a slow post-exit reap to overrun `QUIESCE`, and that cannot be reproduced deterministically. The shared bound is established by construction, since `wait_for_exit` returns the one tail deadline and the drain uses `deadline.min(tail)`.
- [x] 2.2 @integration (agent) existing connector hook tests (`hooks::`) -> 14/14 passed, including the escaped-descendant and cancellation-reaping tests

## 3. Dream settlement and restored strip flags

- [x] 3.1 @integration (agent) `permission_tests::dream_proposed_external_calls_never_prompt_or_dispatch` -> passed; every rejection reads "tool is not offered in this phase", with no file effect and zero A2A hits
- [~] 3.2 @integration (agent) macOS packaged-install fixture with the restored `strip -u -r` flags -> defer: the instrumented embedded-runtime run belongs to coverage CI and was out of scope for this focused pass. Clippy compiled the test target. The flags are byte-identical to c1c1c2f6 (blob 12a3775b), whose record measured the over-cap size under `-x -S`.
- [~] 3.3 @integration (agent) native Windows and Linux hook and runtime suites -> defer: not pushed per instruction. Windows code (the `wait_for_exit` tuple return) is uncompiled locally.

## 4. Live model behavior

- [~] 4.1 @eval (agent) live provider turn with a rewriting pre-turn hook -> defer: the provider-visible request content is fully determined by the runtime projection and pinned by the fake-provider rows 1.1 and 1.2; a live or paid provider evaluation needs user participation and is not run here

## Notes

- **Deliberation `a2a_send`.** Lead decision: a deliberation `a2a_send` settles as an unoffered-tool `Error` without approval, because deliberation never offers it. This is accepted and unchanged.
- **Windows cold start.** Hooks do not receive the ToolHost `$PSHOME` bootstrap, which AGENTS.md scopes to the built-in shell. That bootstrap was not extended to hooks; the docs now tell Windows stock PowerShell hooks to set `timeout_ms`. The available CI evidence is limited:
  - The archived `windows-hook-cold-startup-budget` record shows one windows-2025 first stock PowerShell hook exceeding the 5,000 ms default. Its exact duration was not measured.
  - `shell_warmup.rs` documents cold engine stalls of "tens of seconds".
  - No CI log records the elapsed cold-start time.
