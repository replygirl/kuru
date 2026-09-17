## 1. Core contracts and configuration

- [x] 1.1 Define bounded invocation usage/phase/outcome and non-content context-source/fit types with absent-versus-zero and provenance semantics; verify pure serialization, ordering and invalid-bound tests in `kuru-core`.
- [x] 1.2 Reuse `assumed_context_window_tokens`, add bounded `context_output_reserve_tokens` parser/default validation and published schema parity; verify supported TOML-to-JSON examples and invalid/unknown-key controls.

## 2. Memory-owned operational ledger

- [x] 2.1 Establish and validate the reserved permanent operational Dolt branch at writable open, with only a narrow `UsageLedger` handle; verify real-Dolt reopen, main revision stability and candidate promotion while usage commits occur.
- [x] 2.2 Implement receipted `admit`, ordered `observe`, `settle`, session marker and folded session inspection with bounded records and idempotent/conflicting replay rules; verify uncertainty reconciliation, partial/terminal usage and no double-counting in real Dolt.
- [x] 2.3 Include the ledger in owned close/purge/recovery and document that `memory export` excludes usage; verify lifecycle and live-only export behavior through actual project reopen/purge fixtures.

## 3. Connector effective-request seam

- [x] 3.1 Add the non-content async preflight hook after native `input_items`/body selection under actor-local pending ownership, with a final pre-send fit and retained byte cap; verify a two-call encrypted continuation refuses oversized mandatory context before any HTTP request.
- [x] 3.2 Deliver ordered usage and original terminal report through the fallible stream observer without replacing absent fields with zero; verify partial usage before failure, sink write failure and terminal precedence with native HTTP fixtures.

## 4. Runtime invocation and context policy

- [x] 4.1 Allocate stable unique invocation identities before every deliberation, speaking, consultation, continuation and dream provider call; durably admit, observe and settle through the actor without holding a memory/candidate lock across inference; verify real-Dolt failure, cancellation, rejected dream, reopen and completed-retry controls.
- [x] 4.2 Inventory mandatory instructions/current receipts/tool schemas/native continuation and optional older history, apply model window/output reserve with whole-row omission only, and record actual omitted counts; verify multilingual/tool-heavy and multi-round fixtures against sent requests and unchanged persisted rows.
- [x] 4.3 Project session token totals, frozen API/API-equivalent price estimates, pre-ledger incompleteness and active-request fit/provenance without reading general ledger SQL; verify zero/absent/subset math and catalog-change stability.

## 5. Application presentation and documentation

- [x] 5.1 Add `/cost`, estimated relevant-request context and actual omission notices beside existing status/permissions, retaining narrow-pane controls; verify real PTY frames at 120, 80 and 40 columns plus valid headless JSON.
- [x] 5.2 Document context estimation, configured/built-in assumptions, output reserve, pre-ledger sessions, incomplete usage and subscription API-equivalent estimates in owning docs; verify docs build, format, links and content checks.

## 6. Cross-lane acceptance and delivery readiness

- [x] 6.1 Run the critical real-Dolt candidate/usage, native pending preflight, full invocation and TUI `/cost` acceptance rows in `verification.md`; record actual local commands, counts and observations without claiming unrun native CI.
- [x] 6.2 Run relevant package format/typecheck/lint, strict Cospec and one combined instrumented coverage suite; record at least 90% workspace line coverage and an independent scoped review before archive.
