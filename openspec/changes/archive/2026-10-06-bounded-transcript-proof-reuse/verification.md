# Verification

## 1. Bounded chain proof [critical]

- [x] 1.1 @regression (agent) count metadata loads across the actual 1,025-turn/32-record complete scan before and after correction -> baseline 26802 EXIT101, one selected failure after exact coverage/writer/stale checks and store close, 33,825 metadata lookups; corrected 88674 EXIT0, same 33 pages with identical records/totals and 2,049 metadata lookups (one 1,025-node initial proof plus page work)
- [x] 1.2 @runtime (agent) retain actual forged/stale/fork and other-session writer coverage -> 88674 EXIT0, six selected existing cases passed: long-page reader/writer/stale, selected-chain forged continuation, terminal fork suffix isolation, captured legacy fork prefix, managed main/candidate pages and closing scanner guard; other targets selected zero
- [x] 1.3 @runtime (agent) abort an actual read held after SQL/response validation but before proof publication -> same long fixture in 88674 reached actual post-transaction/final-response pause, aborted and awaited caller cancellation, observed empty independent proof slot, then observed a full 1,025-node cold proof
- [x] 1.4 @integration (agent) verify per-view proof identity and valid forgotten continuation fallback -> same 88674 fixture reads through new clones on each sequential page, confirms independent view/candidate starts cold, and accepts an evicted first continuation after a full 1,025-node walk; managed candidate/main paging remains isolated. Attachment reset, two-page/four-position/2-KiB encoded bounds and oversize/error disabling reuse are source-reviewed; no runtime oversized-proof injection is claimed.

## 2. Owning checks

- [x] 2.1 @unit (agent) run affected memory host/Windows lint/typecheck, docs, format and managed checks -> host 97698, Windows 66786, all-target/all-feature type 92457, docs 4373, format 21996 and managed e03230 all terminal EXIT0; Windows result is cross-target compiler evidence
- [~] 2.2 @e2e (agent) repeat existing transcript PTYs -> defer: UI unchanged; accepted U5 120/80 long-item and held-stream evidence retained unless a changed source concern requires a selected rerun
- [~] 2.3 @runtime (agent) native Windows execution -> defer: hosted CI after normal delivery; local Windows lint/typecheck is compiler evidence only

## Observed scope and limits

Independent Preflight source review cleared frozen store 9bbe2fc3, proof 5375d8c0, RPC add300bd and docs d0519bd8: exact cut checks, bounded issued positions, typed body/privacy validation, fallback, attachment/view lifetime and post-transaction publication. This is source review, not a second test run. Final strict c93a90 passed with zero errors/warnings before archive.

The regression observes actual metadata-query work, not a millisecond SLA. Initial/cache-miss proof remains linear in chain length; unrelated selected-revision changes require fresh proof, and returning toward newer evicted pages can still repeat prefix pages. Cache bounds affect reuse only, and no whole-chain metadata/body cache, schema/index, wire contract or UI changes were introduced.

79336 stopped before libtest at the Cargo/libtest argument boundary; the corrected baseline 26802 actually exercised and failed the work contract. First typecheck 19072 caught a fixture-only wrong candidate method name; correcting it to the existing begin_candidate API produced 92457 success before the scoped native selection. No timeout, privacy, coverage or cleanup assertion was relaxed.
