# Tasks

## 1. Diagnosis record

- [x] 1.1 Record the placeholder semantics and the affected-site list in the proposal and the PR body. `view.busy` selects "Keep your next thought here…"; otherwise the composer shows "What shall we explore or build?". Verify with a grep for `What shall we explore or build?` and `/dream` that every hit is covered or justified.
  - Evidence: job 111067851181 was fetched once. The failure, captured screen and launch arguments are quoted in the proposal. The scoped premise (a startup dream and the 25-backspace site) is contradicted by four facts: `--no-dream`; the `wait_text_with_timeout` message format; the Jungian five-part one-turn state; and the only dream being the smoke's manual `/dream`. The corrected site is `Terminal::command("/dream", None)`'s settle in `smoke`.
  - Grep hits for `What shall we explore or build?` in `tests/terminal.rs` and `tests/support/terminal.rs`:
    - The `/mem` backspace, the `focus draft` backspace and the paste clear are not post-dream state (`--no-dream`, no operation in flight). They are unchanged.
    - `Terminal::command` keeps its settle for non-dream commands.
    - The new `dream-settle` fixture is test-only.
    - `grep '/dream'` finds one hit in `terminal.rs` (now covered) and none in `windows_terminal.rs`.

## 2. Support helper

- [x] 2.1 Add a helper in `apps/kuru-tui/tests/support/terminal.rs` that waits for the dream's completion: the welcome placeholder and `enter send` return, and the status row above the separator no longer leads with `dream · `. Bound it by the passed sandbox startup timeout and put the screen in the expiry diagnostic. Verify the diagnostic contains the captured screen when it expires.
  - Evidence: `Terminal::wait_dream_settled(timeout)` uses `Terminal::wait`, whose expiry embeds `diagnostics()` (the screen). `terminal_driver_awaits_dream_completion_not_a_stale_dream_entry` asserts the expiry error contains `the dream settles: timed out`, the busy status row `dream · pool  ·  3s` and `Keep your next thought here`.
- [x] 2.2 Use the helper at every affected site, and verify no other post-dream placeholder assertion relies on a bare `READY_TIMEOUT` and no assertion after settling changed.
  - Evidence: `smoke` now runs `submit("/dream")` and then `wait_dream_settled(sandbox.startup_timeout)`. `Terminal::command` is now `submit` plus its unchanged settle, so every other command site behaves byte-identically. The following `/unknown` and later assertions are unchanged.

## 3. Evidence

- [x] 3.1 Add a deterministic test in which a screen shows `dream · pool` with the busy placeholder, verify the helper waits and then passes, and record mutation evidence that the old shape fails.
  - Evidence: deterministic test `terminal_driver_awaits_dream_completion_not_a_stale_dream_entry`. Its ack-gated fixture renders the busy dream frame, where a 200 ms helper wait expires with the screen. After the acknowledgment it renders the settled frame, which retains the `dream · pool · …` activity entry and a `pool dream ·` idle status label, and the helper passes.
  - Mutation evidence (local, not committed). After `submit("/dream")` and the busy placeholder, the child was SIGSTOPped for 12 s after `submit("/dream")`.
    - Old shape (`wait_text` welcome placeholder under `READY_TIMEOUT`): returned after 10.12 s with `screen contains ["What shall we explore or build?", "enter send"], excludes []: timed out after 10s; process 98964 is still running`. The screen showed `Keep your next thought here…` and `esc cancel`, the same busy composer state as CI. The pause landed before the dream event, so the status row read `Updating session`; the dream-labelled status row is exercised by the deterministic fixture test. Test FAILED.
    - New shape (`wait_dream_settled(sandbox.startup_timeout)`): returned Ok after 17.69 s. Test ok.
- [x] 3.2 Run `mise run //apps/kuru-tui:test` for the affected PTY tests and record observed results; name unrun checks and reasons.
  - Evidence, local macOS with real PTYs:
    - `mise run //apps/kuru-tui:test -- --test terminal`: with `--all-targets`, this ran every kuru-tui test target. All results ok and none failed. `tests/terminal.rs`: 44 passed, 0 failed, 1 ignored (fixture entry), including `real_pty_accepts_chat_navigation_commands_and_restores_terminal` and the new test.
    - `mise run format:rust` passed.
    - `mise run //apps/kuru-tui:lint` (clippy `-D warnings`) passed.
    - `mise run //apps/kuru-tui:typecheck` passed.
  - Not run:
    - `//apps/kuru-tui:lint:windows`: the changed files are `#![cfg(unix)]` or unix-only test support.
    - Coverage: CI-enforced, with no application code changed.
    - Native Linux and Windows jobs: CI only; there is no local host for them.
- [x] 3.3 Record the CI outcome at the pull request head.
  - Evidence: run 37129537173 at head 57105e8a654745ef6eb44bf00e12d5f987b94934 concluded success (`gh run view`). `gh pr checks 195` reports every check pass except four `dolt-windows-arm64` engine-rebuild jobs, which are skipped by design. No job was rerun.
    - `ci-gate` (job 111227924837), `quality / quality-gate` (111222460862) and `Lint (x86_64-pc-windows-msvc)` (111221842816): pass.
    - `native-tests (macos-latest) / Coverage partition (macos-latest, 4)` (111221843334): `real_pty_accepts_chat_navigation_commands_and_restores_terminal ... ok`, the test that failed in run 37076544504. Partition 2 (111221843347): `terminal_driver_awaits_dream_completion_not_a_stale_dream_entry ... ok`.
    - `native-tests (macos-latest) / Coverage merge (macos-latest)` (111224541026): 94.75% of lines (123688 of 130533), gate 90%.
    - Native Linux and Windows (x64 and arm64) coverage, behavior, installation and update jobs: pass.
  - Inference: one green run does not show the flake is gone, since the original failure depended on runner load; it shows only that nothing regressed.
