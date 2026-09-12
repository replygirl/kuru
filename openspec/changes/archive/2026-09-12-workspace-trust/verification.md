## 1. Authority cannot activate before review [critical]

- [x] 1.1 @integration (agent) invoke real CLI commands with mixed unapproved claims -> `real_cli_commands_enforce_the_documented_claim_matrix_before_side_effects` passed, observing exact refusal categories and no child/socket/file/state activation; reached-undo and snapshot-only config fixtures also passed.
- [x] 1.2 @e2e (agent) invoke stdio and HTTP MCP through the real command path before/after a one-invocation grant -> `stdio_and_http_mcp_require_cli_approval_before_activation` passed: zero child/socket before refusal, then one completed stdio peer and three expected HTTP initialization/list requests after approval, without a persistent record.
- [x] 1.3 @e2e (agent) drive native Unix PTY refusal, EOF and persistent choice -> `real_pty_refusal_eof_and_persistent_choice_precede_the_alternate_screen` passed; refusal/EOF created no approval or alternate-screen runtime, while the explicit persistent choice wrote the complete approval.

## 2. Exact-root and snapshot matching [critical]

- [x] 2.1 @integration (agent) change authority, source, ordinary settings, root identity and path after approval -> exact-path/native-identity/full-manifest CLI fixtures and core manifest provenance tests passed; authority/source/identity/path changes invalidated approval, ordinary edits retained it, and grouped claims retained contributing sources.
- [x] 2.2 @regression (agent) replace ancestor configuration after parsing -> `snapshot_freezes_files_and_keeps_cli_and_remaining_ancestor_claims_distinct` passed, retaining the reviewed configuration and digest.
- [~] 2.3 @runtime (agent) replace the retained workspace before shell and stdio-MCP cwd-based spawn on native Unix and Windows -> defer: native Windows execution requires CI before merge; macOS platform replacement and connector shell/stdio replacement fixtures passed without launching the configured child. Windows fixtures are present; the remaining Unix pathname race is documented.

## 3. Redaction and private approval state [critical]

- [x] 3.1 @integration (agent) exercise config errors and redacted CLI trust/auth paths -> CLI malformed/type/semantic, status/revoke/config and pending/inactive Responses fixtures passed, complemented by core bounded-read/missing-path/UTF-8 tests. Observed output excluded fake values and controls; pending Responses environment remained unread. This is layered evidence, not every input crossed with every command.
- [x] 3.2 @integration (agent) exercise unsafe approval-store objects and publication reconciliation -> five trust-store unit tests passed, including malformed/oversized/hard-linked, symlinked/permissive, and unreconciled-publication cases; none matched approval. Uncertainty evidence combines reconciliation tests with native publication primitives, without claiming approval-layer OS fault injection.

## 4. Command activation matrix

- [x] 4.1 @integration (agent) run the command activation matrix and fixed native login -> the real CLI matrix, non-creating inspection, memory-bootstrap refusal, reached provider-free undo and native-login-bypass tests passed. Undo preserved later chat/session state; native login reached its fixed URL/callback and cancelled without token exchange or unrelated API-key reads.
- [x] 4.2 @unit (agent) construct mixed MCP/external-agent maps and explicit overrides -> core manifest/provenance and hidden-invalid-layer tests passed; effective ancestor leaves remained, overridden/dormant claims were omitted, and CLI overrides did not hide invalid local types.
- [x] 4.3 @eval (agent) evaluate mixed ancestor/explicit/CLI command claims -> the real CLI matrix and snapshot tests passed for applicable categories, explicit shell/provider overrides and ordinary no-prompt settings.

## 5. Documentation and native evidence

- [x] 5.1 @integration (agent) run documented trust commands and inspect rendered references -> isolated CLI/PTY fixtures and owning docs checks passed; rendered configuration/commands/tools pages were manually checked for matching syntax, full persistent versus one-command grants, pure inspection, and process-authority limits.
- [x] 5.2 @integration (agent) run owning static/docs checks and coordinated workspace coverage -> macOS final coverage exited 0 in 218.40 seconds, 17,284/17,887 lines (96.63%), with trust.rs in the fresh report; format/lint/typecheck/tooling/docs checks exited 0. Windows execution is deferred in row 2.3 and the evidence below.

## Observed evidence and remaining gates

2026-09-12, native macOS: the logged combined coverage attempt completed with
exit 101 (`/private/tmp/kuru-final-coverage.log`), so it establishes no passing
workspace coverage percentage. Its ten trust integration tests passed, including
both MCP transports before/after approval and reached provider-free undo on real
Dolt state. Retained-root shell/stdio rejection, actual live-undo cancellation
and reconciliation, and provider-free runtime undo also passed in that run.

The failing targets were TUI CLI/auth/preferences/terminal, connector unit tests,
and core config tests. Their owners are preserving the original state/cleanup
guarantees while updating obsolete diagnostic and config-inspection oracles.
The connector fixture's invalid relative retained root was corrected; its focused
protocol/version/pagination test then passed through its owning mise task.
Core now validates the explicit local layer before CLI overrides; all 21 config
tests passed, including hidden-invalid-local-type and external-agent provenance
regressions. Core lint/typecheck also passed with explicit exit 0.

Subsequent `mise run //apps/kuru-tui:test` completed with exit 0 in 95.22 seconds,
including all 14 trust integrations and the repaired CLI/auth/preferences/terminal
targets. `real_cli_commands_enforce_the_documented_claim_matrix_before_side_effects`
exercised the command/category matrix, exact refusal labels, ordinary defaults,
CLI overrides, workspace-write refusal and external-agent visibility without
unapproved child, socket or file mutation. Real native login reached its fixed
authorization URL under malformed workspace configuration and an unreadable
Responses environment, then cancelled without browser launch or token exchange.
Auth before/after one-time approval and inactive-route non-read were exercised.
TUI lint/typecheck and core lint/typecheck also completed with explicit exit 0.

The config/parser evidence combines real CLI parse/type/semantic-error cases
with core bounded-read, missing-path, malformed UTF-8 and per-layer validation
tests; it does not claim a dedicated CLI permission-denied fixture. Approval
state evidence combines explicit unknown-field, malformed, oversized, symlink,
hard-link and permissive-object fixtures with reconciliation-mismatch tests and
the checked native publication primitives. It does not claim a real OS uncertain
publication was injected at the approval-store layer.

Rendered output from the successful `docs:check` build was inspected at
`reference/configuration.html`, `reference/commands.html` and
`reference/tools.html`. Its command syntax, complete persistent approvals,
one-command nonpersistent grants, inspection behavior and process-authority
limits match the exercised CLI cases. This is observed rendered-content review,
not a claim that an automated test compares every prose statement.

Windows retained-root and ConPTY decline/persistent-choice tests are present but
native execution is deferred to Windows CI before merge. Unix PTY EOF is covered;
no Windows EOF execution is claimed. The second logged combined run exited 101
with one existing desktop-handoff fixture race: an empty marker was observed
between creation and completed write. The corrected fixture waits for completed
content within the same two-second bound and passed its focused test. The final
replacement combined run (`/private/tmp/kuru-final3-coverage.log`) exited 0 in
218.40 seconds. Its newly generated LCOV includes trust.rs and reports
17,284/17,887 covered lines (96.63%). Final format check also exited 0. The earlier
batch's 96.84% report is not used for this change. Private logs and the
coordination ledger are not published docs.
