# Proposal

## Why

Measured: in PR #181, run 37076544504 (head da4033ed on main f958287a; the PR touches only kuru-runtime tests), macos-latest coverage partition 4, job 111067851181, `real_pty_accepts_chat_navigation_commands_and_restores_terminal` in `apps/kuru-tui/tests/terminal.rs` failed with `screen contains ["What shall we explore or build?", "enter send"], excludes []: timed out after 10s; process 12677 is still running`. The captured screen showed the status row `⠋ dream · pool  ·  10s`, the activity panel headed `dream · pool · parts are con…`, the composer placeholder "Keep your next thought here…", the busy dock `esc cancel`, `✧ Jungian`, `PARTS / 5` and `1 turns`. The launch arguments included `--no-dream --mode freudian`.

Correction to the scoped premise: this was not a startup dream. The fixture launches with `--no-dream`. `kuru-core/src/config.rs` maps that flag to `dream_every = 0` and `dream_on_exit = false`, and automatic dreaming runs only when `dream_every > 0` (`kuru-runtime/src/engine.rs`). Nothing starts an operation at startup. The error text comes from `wait_text_with_timeout`. It does not come from `wait_composer_frame`, which the 25-backspace site uses. The Jungian five-part, one-turn state is reachable only in `smoke(.., full = true, ..)` after `hello from a terminal` and `/mode jungian`. In that sequence the only dream is the manual `terminal.command("/dream", None)`. `Terminal::command(_, None)` ends with exactly the failing wait, under `READY_TIMEOUT`. The status row's `10s` is `operation_ms / 1000` since that operation began.

Inference: the failing wait was the settle wait of the manual `/dream`. Under coverage on a loaded runner, the dream had not finished 10 s after Enter. This is inferred from the facts above; the log has no stack trace naming the line. The change slug keeps its scoped name, though the dream is the manual `/dream`, not a startup dream.

Deviation from the scoped observable: the scope asked to wait for the activity line to leave `dream · pool`. The activity panel is a retained log that keeps that entry after completion (the CI screen and the fixture both show it), so it never leaves. The helper instead waits on the status row and the composer and dock state, which change exactly when the dream's operation completes. This is the equivalent observable, chosen deliberately.

Source semantics: `src/ui/render.rs` shows the empty-composer placeholder "Keep your next thought here…" while `view.busy` and "What shall we explore or build?" otherwise. The dock shows `esc cancel` while busy and `enter send` otherwise. The busy status row is `{spinner} {status}  ·  {secs}s`. `Event::Dream` sets the status to `dream · <actor>` unless completion is locked (`src/ui.rs`), so a later event can replace it while still busy. The activity panel keeps the `dream · …` entry after completion, and the idle status row can show `… <actor> dream · …`. The welcome placeholder therefore holds only once the dream's operation has completed. Waiting for it under a flat 10 s guesses the runner's speed instead of awaiting that event.

## What Changes

- `apps/kuru-tui/tests/support/terminal.rs`: split `Terminal::command` into `submit` (idle, type, observe the draft, Enter) plus its unchanged picker or welcome settle under `READY_TIMEOUT`. Add `wait_dream_settled(timeout)`. It ends when the composer shows the welcome placeholder, the dock shows `enter send`, and the status row above the separator no longer leads with `dream · `. `wait` puts the captured screen in the expiry diagnostic.
- `apps/kuru-tui/tests/terminal.rs`: the smoke's `/dream` now runs `submit("/dream")` and then `wait_dream_settled(sandbox.startup_timeout)`. This reuses the existing sandbox budget; no new constant is added. A new ack-gated `dream-settle` PTY fixture and the test `terminal_driver_awaits_dream_completion_not_a_stale_dream_entry` cover the helper.
- Unchanged, with justification: the welcome-placeholder waits after the `/mem` backspace, the `focus draft` backspace and the paste clear. They run under `--no-dream` with no operation in flight, so they are not post-dream state. `windows_terminal.rs` has no `/dream`.
- No change under `apps/kuru-tui/src`. `READY_TIMEOUT` is not raised and no retry is added. Assertions after the dream has settled are unchanged.

## Impact

Only test and test-support code in `apps/kuru-tui` changes. CI time is unchanged when the dream finishes promptly. When the dream outlasts 10 s, the smoke waits for it up to the existing sandbox startup budget instead of failing.
