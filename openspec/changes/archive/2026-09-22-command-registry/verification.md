## 1. Registered commands agree across help, completion and dispatch [critical]

- [x] 1.1 @e2e (agent) drive a real PTY through `/help`, ambiguous and unique `Tab` completion, a working built-in, and an unknown future slash name -> completed names match help and dispatch, while the unknown name never reaches the fake provider.
- [x] 1.2 @unit (agent) enumerate registry descriptors and parser/completion results -> names are unique, prefix order is stable, and unimplemented P09/P11/P12 names and internal review tokens are absent.

## 2. Clear remains a view-only operation [critical]

- [x] 2.1 @e2e (agent) run a fake-provider real PTY conversation, `/clear`, another turn, and a same-session restart -> old visible rows disappear only on the current surface; persisted history and next-turn context remain, with no tool/provider call for `/clear`.
- [x] 2.2 @unit (agent) clear a populated `View` -> transcript rows, row metadata and scroll reset while session, nonempty cached usage, turn count and selected focus stay unchanged; the real-PTY continuation separately proves stored history stays available.

## 3. Status and modal input preserve current authority [critical]

- [x] 3.1 @e2e (agent) request `/status` in a real PTY with a fake provider and current session selections -> the visible projection names session/project/model/effort/mode/focus and known usage without an extra provider request.
- [x] 3.2 @integration (agent) exercise permission and instruction review input plus idle composer digits and `Tab` -> modal choices retain priority, ordinary digits remain text, and completion never grants permission or starts a turn.

## 4. Owning checks and published guidance

- [x] 4.1 @integration (agent) run owning TUI granular tests, lint, typecheck, format, docs checks, strict Cospec validate/apply, and diff checks -> all pass; normal pre-push coverage and native PTY CI results are recorded separately.
- [x] 4.2 @equivalence (agent) review updated `docs/usage.md` and published command reference against the registry -> only working names are documented and `/clear` explicitly retains stored history.

Observed 2026-09-22: `mise run //apps/kuru-tui:typecheck` and `//apps/kuru-tui:lint` passed with the verified offline Dolt bundle mirror. Focused registry tests passed 2/2; View command completion/modal-priority and local clear/status tests each passed 1/1, including the final nonempty cached-usage assertions. The real-PTY fake-provider fixture passed 1/1 after the final status-field and unique-`Tab` assertions. Its first native run exposed that the clear notice was hidden by the empty welcome scene; `/clear` now uses the existing transient notice surface and the successful rerun observed that completed frame. A restricted-sandbox PTY attempt returned `Operation not permitted`; the successful native runs used the authorized unsandboxed profile. The fixture observed no extra provider request for unknown `/compact`, `/status` or `/clear`, saw the older turn in follow-up provider input after clearing, and saw it again on same-session resume. Independent source review cleared catalog/dispatch/modal/clear/status behavior and the notice correction. `mise run //apps/kuru-docs:check` passed after formatting the command reference, including build, local link/content and formatter/lint checks. `mise run format:check`, `mise run cospec:managed:check`, strict Cospec validation and the actual apply gate passed; `git diff --check` passed. Normal pre-push coverage and native CI remain delivery evidence, not claimed here.

Post-archive, the new living `command-registry` spec's generated Purpose placeholder was replaced with its concrete capability description. `mise run cospec:validate` then passed across all active specifications with 0 errors and 0 warnings.
