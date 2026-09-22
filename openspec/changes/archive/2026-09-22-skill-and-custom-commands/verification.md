## 1. Startup catalog reveals approved metadata, not skill bodies [critical]

- [x] 1.1 @integration (agent) parse checked project and user skill fixtures with valid, malformed, shadowed, oversized and replaced metadata -> only effective bounded name/description reaches the snapshot; body/reference markers stay unread and absent from the prompt
- [x] 1.2 @e2e (agent) drive an isolated fake-provider invocation through ordinary workspace preflight -> the first request carries approved metadata only, with project source claims in the reviewed manifest
- [x] 1.3 @eval (agent) run a bounded task-to-skill selection corpus through the normal already-authenticated provider route when available -> record which metadata led to `skill_load`, with disclosed prompt sources and any route/unavailability limitation; do not claim broad model-selection accuracy

## 2. On-demand skill selection preserves exact prompt authority [critical]

- [x] 2.1 @integration (agent) select body then one reference through the actor tool and existing once/persist/deny review -> each exact source enters a complete manifest before the next provider request and unselected references remain absent
- [x] 2.2 @integration (agent) replace a checked source or directory, revoke/reapprove the root during a pending review, and deny a selection -> no stale approval, partial prompt update, body disclosure or side effect occurs
- [x] 2.3 @integration (agent) request project material headlessly without a grant and with `--trust-workspace-once` -> the first reports required trust while the explicit one-invocation grant applies only to that command

## 3. Skill declarations do not bypass tool permissions [critical]

- [x] 3.1 @integration (agent) select a skill declaring `allowed-tools` and a shell action, then attempt a denied tool call -> selection grants no permission and the ordinary permission decision still refuses the action

## 4. Custom commands agree across help, completion and dispatch [critical]

- [x] 4.1 @unit (agent) construct built-in/project/user collision and malformed-entry catalogs -> deterministic effective names, descriptions and source claims
- [x] 4.2 @e2e (agent) use a real PTY and fake provider to open help, complete a custom slash prefix, invoke it with literal arguments, and submit an unknown slash -> one ordinary same-session provider turn contains the captured prompt and arguments, with no provider turn for the unknown name
- [x] 4.3 @integration (agent) verify project command preflight and captured-byte immutability across source edits -> unapproved names are hidden and later file changes do not alter the current invocation's prompt

## 5. Package and documentation checks

- [x] 5.1 @regression (agent) run owning core/connectors/runtime/TUI tests, lint, typecheck, Rust format, docs check, strict Cospec validation, apply, managed-file check and diff check -> each required scoped check exits successfully or records a named native limitation
- [~] 5.2 @integration (agent) run the normal non-bypassed commit and push hooks through delivery -> defer: archive must precede the branch commit; Delivery will run the normal push hook and record coverage and native CI against the exact delivered head.

## Observed local evidence (before archive)

- `mise run //packages/kuru-core:test` passed after correcting the new integration fixture to call the public `AuthorityClaim::category()` accessor. The pass included the five then-current `prompt_sources` tests; the subsequently added project-skill precedence/oversize fixture still needs its focused rerun.
- `mise run //packages/kuru-connectors:test` first compiled but 78 loopback fixtures failed with sandbox `Operation not permitted` at listener binding. The same owning task with loopback access passed all 208 connector library tests; this was an execution-permission difference, not a product change.
- `KURU_DOLT_BUNDLE_DIR=/private/tmp/kuru-phase1-bundles KURU_DOLT_BUNDLE_OFFLINE=true mise run //apps/kuru-tui:typecheck` passed all targets using the manifest-matching verified offline archive. The first unmirrored attempt stopped in bundle preparation on restricted DNS before Rust checks.
- The first native `mise run //apps/kuru-tui:test` compiled with the owning prefetch/supervisor and passed the new real-PTY custom-command and skill-review tests, but failed four instruction-gate unit fixtures and one inherited navigation PTY assertion. The gate fixtures had passed an existing temporary root where the approval store expects its private child; the new generation check also needed to preserve an explicit Once when no readable stored generation exists. The navigation assertion expected lowercase text while the existing UI displays `Unknown command`. After the narrow corrections, the six focused gate tests, navigation PTY, custom-command PTY and skill-review PTY each passed. The subsequently strengthened sequential reference/Deny fixture awaits its focused rerun.
- `mise run //packages/kuru-core:lint`, `mise run //packages/kuru-connectors:lint`, and the corrected `mise run //apps/kuru-tui:lint` passed. The first TUI lint found an eight-argument private dispatch helper; grouping its two existing review senders cleared Clippy without changing routing.
- `mise run format:check` passed with host macOS dynamic-store access after sandboxed Taplo panicked before checking files; Rust, TOML, connector fixture and docs formatting all passed. The final spec wording edit passed `mise run cospec -- validate skill-and-custom-commands --strict`, `mise run cospec -- apply skill-and-custom-commands --json` (gate clear), `mise run cospec:managed:check`, and `git diff --check`.
- On the final source, `mise run //packages/kuru-core:test` passed all six prompt-source fixtures; the six focused instruction-gate tests and the real-PTY skill continuation passed with the owning prepared supervisor. `mise run //packages/kuru-runtime:test` passed 143/143 real-Dolt tests in 209.12 seconds. `mise run //apps/kuru-tui:lint`, `mise run docs:check` (build, links, anchors, lint and formatting), and the current binary build passed. The first full TUI suite's remaining targets passed; its five fixture/assertion failures were corrected and verified by the focused native reruns above, so normal combined coverage remains the final broad check.
- Three benign live `codex` subscription requests used model `gpt-5.6-luna`, the isolated temporary project, its three disclosed skill descriptions, an explicit one-invocation workspace grant and the existing authenticated `/private/tmp/kuru-preview/data` route. The short code-review task completed without `skill_load` (its default pool work reported 26,151 input and 6,312 output tokens). With `max_rounds=1` and `max_tool_calls=3`, the implementation-outline task called `skill_load` with `name=outline` and settled `ok` (20,072 input, 1,843 output tokens); the copyedit task similarly selected `name=copyedit` and settled `ok` (22,847 input, 469 output tokens). The JSON event receipts establish selected names/outcomes, not exact prompt-use quality; the fake-provider and real-PTY fixtures establish the body/reference boundary. This is a preliminary single-model, three-task check with one nonselection, not a general accuracy claim. No credential store was inspected or copied.
