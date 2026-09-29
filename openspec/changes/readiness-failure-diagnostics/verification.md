# Verification

All local runs: macOS arm64 (Apple M3 Max, 14 cores), debug test profile, shared
host. Load averages are recorded per run.

## 1. The readiness deadline error reports the client phase split [critical]

- [x] 1.1 @regression (agent) `service::tests::readiness_deadline_reports_the_client_phase_split` on unchanged product code: a real `attach_or_start` launch of an owner script that stays alive without publishing an endpoint, `startup_timeout_secs = 1` -> fails before the change because the error lacks `client phases:`; passes after with the leading text intact, phases summing to at least 1000 ms and no more than the observed wait, readiness holding most of it, `(polls - 1) * 100 <= readiness`, `child=running` and `last-attach=no-endpoint` -> observed 2026-09-29. Before (load 13.74): FAILED, `readiness failure lacks the client phase split: memory service readiness deadline exceeded`. After (load 6.39): ok. Then 20 more runs, 10 without and 10 with `KURU_TEST_MEMORY_STARTUP_STAGES=1` (load 8.2-8.3): 20/20 ok, each about 1.02 s. Sample text: `memory service readiness deadline exceeded; client phases: election=0ms; owner-probe=1ms; spawn=1ms; readiness=1025ms; polls=11; child=running; last-attach=no-endpoint`. With the stage diagnostic the existing observations stay in place before the split: `memory service readiness deadline exceeded; test startup observations: stage=none; owner=free; endpoint=absent; client phases: ...`
- [x] 1.2 @unit (agent) `service::tests::readiness_split_text_is_stable` -> exact text for fixed instants, including consecutive offsets that sum to the total -> observed ok locally (12.7/15.9/55.2/1000.4 ms instants render as 12/3/40/945 ms, summing to 1000)
- [x] 1.3 @integration (agent) `mise run //packages/kuru-memory:test` -> the whole memory suite passes, including the existing matchers on `did not publish a readable endpoint`, `existing memory service owner did not publish` and `memory service owner is still active` -> observed exit 0 in 503 s (load 12.27 at start, 6.10 at end): lib 293 passed, 0 failed, 0 ignored; the other targets 12, 10, 5 and 1 passed

## 2. The contained starter fixture surfaces its child's stderr [critical]

- [x] 2.1 @regression (agent) `service::tests::starter_wait_surfaces_the_exited_child_stderr` (Unix, shared wait helper) with the helper's stderr tail removed -> fails; with it restored -> passes for both an exited child and a child that misses its deadline -> observed. Tail removed (load 13.74): FAILED, `starter exit discarded the child's stderr: contained memory starter exited before readiness: exit status: 1`. Restored: ok, and 20/20 ok in the loop above
- [x] 2.2 @integration (agent) `mise run lint:windows` -> the Windows-only fixture wiring and `contained_starter_failure_reports_its_stderr` compile and lint for the Windows target -> observed exit 0; `cargo clippy -p kuru-memory --target x86_64-pc-windows-msvc --all-targets --all-features --locked -- -D warnings` finished clean. This proves it compiles, not that it runs
- [~] 2.3 @runtime (agent) Windows CI runs `contained_starter_failure_reports_its_stderr` and the two contained starter tests, expecting the starter's `service supervisor path must be absolute` cause in the failure text; only hosted Windows can prove this -> defer: no push or CI run is authorized for this change yet; the first Windows CI run on the PR is the evidence

## 3. Static checks and documentation

- [x] 3.1 @integration (agent) `mise run lint`, `lint:windows`, `typecheck`, `format:check`, `lint:tooling`, `docs:check` and `cospec validate readiness-failure-diagnostics --strict` -> each passes -> observed exit 0 for each: `format:check` 5 s, `lint` 88 s, `lint:windows` 60 s, `typecheck` 48 s, `lint:tooling` 2 s, `docs:check` 4 s; `cospec validate --strict` passed
- [x] 3.2 @manual (agent) read the rendered sentence in `docs/configuration.md` and `apps/kuru-docs/reference/configuration.md` -> it states the error keeps its leading text and appends the split, and makes no claim about owner stages -> observed in the diff; both pages name the leading text, the split fields and the last-poll outcome only
