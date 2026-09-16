## 1. Projected semantic contract [critical]

- [x] 1.1 @integration (agent) exercise engine broadcast, trace, completed output, JSON adapter, and TUI with fake-secret event payloads -> every path carries the matching typed variant or fixed withheld event, emits exactly three wire keys, and contains no fake secret
- [x] 1.2 @e2e (agent) drive the typed TUI event fixtures through the runtime view path -> peer recipient, relationship, state, speaker, and response activity remain useful without rendering peer message bodies

## 2. Settled tool receipt behavior [critical]

- [x] 2.1 @integration (agent) drive cognitive and external successful, error, denied, budget-failed, and cancellation tool paths -> each admitted call yields one settled observation with the required classification and no result digest when no receipt exists
- [x] 2.2 @integration (agent) recompute argument bytes and result digest from projected fixture JSON, including truncation and projected errors -> recorded values equal serialized projected UTF-8 bytes and SHA-256 while raw result and diagnostic secrets are absent

## 3. Journal compatibility and retry [critical]

- [x] 3.1 @runtime (agent) checkpoint, reopen, and replay a completed v2 turn through the memory fixture -> typed events and compatible wire JSON survive without provider/tool dispatch or another settled observation
- [x] 3.2 @integration (agent) replay v1, malformed v2 known, and unknown historical event fixtures -> v1 retains projected legacy behavior, malformed known records become withheld, and unknown historical kinds become projected legacy events
- [x] 3.3 @runtime (agent) execute completed exact retry and possible-dispatch retry fixtures -> exact retry returns stored output with zero dispatch and possible dispatch remains refused

## 4. Regression and documentation

- [x] 4.1 @regression (agent) run focused package-owned runtime and TUI tests -> event, journal, engine, and UI consumers pass
- [x] 4.2 @integration (agent) run documentation checks -> documented wire, measurement, and replay compatibility guidance builds and links

## Observed verification

- Focused package-owned runtime checks passed for semantic wire validation, projected secret handling, successful/error/denied/budget/cancelled settlement cardinality, peer-consultation settlement timing, v2 memory reopen/exact retry, and possible-dispatch refusal. `//packages/kuru-runtime:lint` and `//packages/kuru-connectors:lint` passed.
- The delegated TUI migration passed its package typecheck, activity/visual routing checks, real-memory privacy fixture, and real-PTY cancellation/generation-fence fixture. The retained-log coverage recovery also passed its focused diagnostic-ring fixture.
- `mise run docs:check` passed. The final retained-log `mise run coverage` passed with exit 0 in 534.54 seconds and wrote `target/coverage.lcov`; the runtime crate reported 98 passed tests.
- Independent scoped review cleared the final source, including strict v2 replay validation, bounded projection, settled observation timing/cardinality, and typed TUI consumption.
