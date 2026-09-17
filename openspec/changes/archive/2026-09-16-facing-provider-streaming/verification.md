## 1. Native stream and final typed authority [critical]

- [x] 1.1 @integration (agent) Chunked HTTP fixtures for both native routes, UTF-8/refusal/summary/tool fragments and opaque continuation -> connector held-terminal API and subscription fixtures observed `TextDelta` before terminal; reconciliation tests cover UTF-8, refusal, visible summary, tool fragments, sparse raw indexes and private continuation.
- [x] 1.2 @integration (agent) Conflicts, terminal-only success, duplicate buffered terminal, EOF, bounds, sink failure and cancellation -> focused connector SSE tests cover terminal-only success, buffered duplicate terminals, mismatched fragments/types, EOF, bounds and failed outcomes; runtime/tool-loop cases establish no partial dispatch and cancellation.
- [x] 1.3 @unit (agent) Collector usage and terminal contract -> collector tests preserve absent versus zero/cache/reasoning values, expose raw terminal usage to observers, retain terminal-field precedence, and reject missing/duplicate/failed terminals.

## 2. Selected-facing runtime isolation [critical]

- [x] 2.1 @runtime (agent) Real Dolt part and relationship speaker fixtures with private peer/consultation/dream sentinels -> runtime owner observed 4/4 real-Dolt progress cases for part and relationship speakers; private deliberation, consultation and dream sentinels stayed absent and summaries remained transient.
- [x] 2.2 @runtime (agent) Multi-round tools, cancel, completed retry and resumed interrupted session -> runtime owner observed round replacement, no partial tools, cancellation/resume and exact completed retry.
- [x] 2.3 @eval (agent) Scripted provider proposes fragmented tool output during a selected speaking round -> runtime owner observed final typed-call authority with no dispatch from provisional fragments.

## 3. Interactive preview and final-only CLI [critical]

- [x] 3.1 @e2e (agent) Delayed native fixture through real PTY at 80x24 and 120x40, resize 40x18 -> app owner observed partial preview and visible summary/activity before one final answer at all specified sizes.
- [x] 3.2 @e2e (agent) Burst/drop, cancellation and late operation updates through real terminal -> app owner observed a 16 KiB/128-delta burst, summary/truncation cues, responsive composer, cancellation and new-turn fencing; terminal output contained no private/native sentinels.
- [x] 3.3 @integration (agent) Scripted output and exact completed retry -> app owner observed final-only CLI output with cancellation/new-turn/retry behavior and one settled answer.

## 4. Repository verification and independent review

- [x] 4.1 @regression (agent) Relevant format, lint, typecheck, docs and strict cospec checks -> docs check passed (DOCS_EXIT=0, 26.95s); Rust format passed (`P5_FORMAT_RUST_FINAL_EXIT=0`), workspace typecheck passed (`P5_TYPECHECK_FINAL_2_EXIT=0`), tooling lint passed (`P5_TOOLING_FINAL_EXIT=0`), and strict/managed Cospec passed (`P5_COSPEC_STRICT_FINAL_EXIT=0`, `P5_COSPEC_MANAGED_FINAL_EXIT=0`). Root aggregate lint passed in 20.06s (`LINT_FINAL_EXIT=0`, `/private/tmp/kuru-phase1-streaming-lint-final.log`) after rerunning docs setup with access to its normal repository hook configuration; the earlier sandbox-only failure is retained in the prior log.
- [x] 4.2 @regression (agent) Single combined instrumented coverage suite -> recovery coverage passed (`P5_COVERAGE_FINAL_EXIT=0`) in 543.46s with 37,487/39,819 lines (94.14%) and saved `target/coverage.lcov`; the preserved first run exited 101 solely on the migrated embedded-runtime JSON fixture.
- [x] 4.3 @equivalence (agent) Independent final diff review against Phase 1 and stream/privacy contracts -> runtime, app and connector independent reviews cleared; connector review findings on terminal usage precedence, raw identities/types, terminal authority and delayed route observation were resolved.
