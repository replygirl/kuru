# Tasks

## 1. Exact fixture diagnosis

- [x] 1.1 Revert unsupported OS5 continuation and held-root case together; retain precise operation context and original removal budget in server_lifecycle.rs.
- [x] 1.2 Execute eight isolated sequential original retained-supervisor iterations on Windows and one elsewhere; qualify every error by iteration/operation and terminate immediately on error.
- [x] 1.3 Verify the original host lifecycle case and closing guard, affected host/Windows static checks, and independent source review before the diagnostic commit.

## 2. Native evidence and completion

- [x] 2.1 Inspect actual Windows CI evidence for the unchanged retained-supervisor lifecycle path; distinguish observed phase failures from unknown historical cause and keep auto-merge off during investigation.
- [x] 2.2 Record the supported final outcome and complete final validation/archive readiness without a causal-resolution claim; actual archive must precede the final branch commit and merge.

## Observed evidence

- Native PR241 at a1ebb failed the new held-root premise on both Windows x64 and ARM: actual `try_exists` returned `Ok(false)` while the handle was held. The original retained-supervisor Windows5 case passed. The prior archive remains immutable; its pending native assumption is disproved, not silently revised.
- New diagnostic strict validation and apply both exited 0; all three returned context files were read before source edits. No active blockers were listed.
- Frozen source SHA256: 62a1de46bf30b57edbe1f3847710b68632f735444749aaf1a88a506802038086. Independent Architecture source review cleared exact ownership, bounded iteration and immediate error behavior.
- Owning prepared host selection 73582 exited 0: original retained lifecycle 1/1 passed in 1.72s and closing-scope guard 1/1 in 0.03s; unrelated cases filtered.
- Host lint 34776, Windows all-target/all-feature lint 11970, typecheck 6614, formatting 24441, docs build/content/link check 60442 and managed check 15268 all exited 0.
- Exact diagnostic head `2da8842dd79ea15fb88473ffe3d4fa1e087ebdc9` passed full CI `37473157914`: terminal SUCCESS with 62 successful jobs and four expected skips. Windows x64 coverage partition5 job `112302384148` and Windows ARM behavior partition5 job `112302516718` both passed the original retained-supervisor case, each executing eight fresh isolated iterations. Their lifecycle targets passed 3/3 in 19.23s and 15.50s respectively; no iteration or operation error was emitted. Thus sixteen fresh Windows iterations passed; this is diagnostic acceptance, not proof of the historical failure's cause or cure.
- The unsupported OS5-continuation policy and disproved held-root mini-case are removed together. Production cleanup, confirmed namespace-absence semantics, immediate contextual error propagation, the original 15-second removal deadline and 25ms observation interval remain unchanged. Native host coverage and platform/update gates passed in the same full run. Historical main `e0885b0f` OS5 operation and cause remain unknown; no unchanged-run retry, guessed production correction, coverage relaxation or paid call occurred.
- Root engineering review authorized truthful diagnostic delivery after full CI, actual archive and final-head checks, followed by all exact-main checks. PR auto-merge remains off during this finalization. Final archive, normal final commit/push hooks, final-head CI, merge, exact-main checks and release remain pending until actually observed; this record does not pre-claim those outcomes.
- Final evidence settlement passed strict validation with zero errors and warnings before marking archive readiness complete. The finalization changes only this active task record; frozen lifecycle source SHA256 remains `62a1de46bf30b57edbe1f3847710b68632f735444749aaf1a88a506802038086`.
