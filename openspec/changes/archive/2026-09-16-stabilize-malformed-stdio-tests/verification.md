## 1. Malformed stdio lifecycle observations [critical]

- [x] 1.1 @regression (agent) add a trailing read while retaining the connector `.done` assertion, observe the focused test fail, then assert one process with one initialize-request transcript -> RED: `mise run //packages/kuru-connectors:test -- catalog_keeps_healthy_http_alias_when_stdio_alias_fails_in_either_order` failed with zero `.done` markers versus one expected and one recorded initialize request; GREEN: the same focused test passed 1/0 with both alias orders, the healthy HTTP tool, unavailable stdio status, exact one-request transcript, and successful shutdown unchanged
- [x] 1.2 @regression (agent) add a trailing read while retaining the CLI `.done` assertion, observe the focused test fail, then assert four one-request initialize transcripts -> RED: `mise run //apps/kuru-tui:test -- direct_tools_keeps_stdout_json_and_reports_filtered_failed_stdio` failed with zero `.done` markers versus four expected; GREEN: the same focused test passed 1/0 with exactly four one-initialize transcripts and all existing stdout, degradation, event, diagnostic-redaction, and synchronous cleanup assertions retained

## 2. Scoped static verification

- [x] 2.1 @unit (agent) run connector and TUI package formatting, typecheck, and lint checks for the settled two-file test change -> `mise run //:format:rust`, connector typecheck/lint, TUI typecheck/lint, and `git diff --check` passed; no broad suite or coverage run was duplicated
