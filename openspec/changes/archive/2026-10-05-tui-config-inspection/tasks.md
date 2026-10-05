# Tasks

## 1. Safe captured configuration projection

- [x] 1.1 Add a bounded `ConfigSnapshot` inspection projection with final-leaf provenance, default-source labels, and saved-preference provenance only for values the overlay applies; verify exact winners across layered fixtures and unchanged runtime finalization. Evidence: all 9 `config_display_projection_tests` passed, including source precedence, preference winners, and captured-byte immutability.
- [x] 1.2 Redact known MCP environment and static header values, URL userinfo, and sensitive URL query values from both TUI projection and parseable CLI snapshot TOML while preserving configured environment-reference names such as `api_key_env`, `token_env`, and `header_env`; verify fake-secret exclusion and benign capability-name preservation. Evidence: the 9 core projection tests passed, including serialized parseable CLI TOML and secret/reference sentinels.
- [x] 1.3 Bound projected rows and text with explicit omitted/truncated counts; verify deterministic ordering and counts at each limit. Evidence: the core bounds test and TestBackend renderer test passed with visible omitted/truncated notices.

## 2. Local TUI command and live state

- [x] 2.1 Register `/config` with no arguments in parsing, help, completion, and dispatch; verify the registry contract tests cover every surface. Evidence: `catalog_names_are_unique_sorted_and_parse_to_their_dispatch_ids` passed with `/config` included.
- [x] 2.2 Pass the captured safe projection and typed initial live selections into the TUI view; verify the command renders from stored data without harness dispatch, memory mutation, provider request, or config reread. Evidence: the TestBackend renderer and synchronized PTY tests passed; the PTY mutated its source file after startup and retained captured values.
- [x] 2.3 Update live mode/model/effort display only after successful selection operations; verify a refused selection leaves prior live values and provenance unchanged. Evidence: synchronized PTY verified successful/refused mode outcomes; real dispatch fixture verified successful/refused mode, model, and effort operations; the renderer verified typed live selection display.
- [x] 2.4 Exercise `/config`, source mutation after launch, selection success/failure, and secret redaction through synchronized real PTY frames; verify oversized-output bounds through the core projection and visible TestBackend frame as recorded in `verification.md`.

## 3. User documentation and integration acceptance

- [x] 3.1 Document `/config`, final-leaf provenance, redaction, the captured configuration versus live selection distinction, and its read-only limits in the command reference; `mise run //apps/kuru-docs:check` passed after applying its Markdown formatter.
- [x] 3.2 Complete each verification ledger row with observed evidence or an explicit defer reason, then run strict Cospec validation and the actual apply gate; the verification rows now distinguish PTY evidence from deterministic oversized-output evidence.
