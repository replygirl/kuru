# Tasks

## 1. Exact fixture diagnosis

- [x] 1.1 Revert unsupported OS5 continuation and held-root case together; retain precise operation context and original removal budget in server_lifecycle.rs.
- [ ] 1.2 Execute eight isolated sequential original retained-supervisor iterations on Windows and one elsewhere; qualify every error by iteration/operation and terminate immediately on error.
- [x] 1.3 Verify the original host lifecycle case and closing guard, affected host/Windows static checks, and independent source review before the diagnostic commit.

## 2. Native evidence and completion

- [ ] 2.1 Inspect actual Windows CI evidence for the unchanged retained-supervisor lifecycle path; distinguish observed phase failures from unknown historical cause and keep auto-merge off during investigation.
- [ ] 2.2 Record the supported final outcome, validate and archive before the final branch commit and merge; do not claim causal resolution from compilation or a green diagnostic run alone.

## Observed evidence

- Native PR241 at a1ebb failed the new held-root premise on both Windows x64 and ARM: actual `try_exists` returned `Ok(false)` while the handle was held. The original retained-supervisor Windows5 case passed. The prior archive remains immutable; its pending native assumption is disproved, not silently revised.
- New diagnostic strict validation and apply both exited 0; all three returned context files were read before source edits. No active blockers were listed.
- Frozen source SHA256: 62a1de46bf30b57edbe1f3847710b68632f735444749aaf1a88a506802038086. Independent Architecture source review cleared exact ownership, bounded iteration and immediate error behavior.
- Owning prepared host selection 73582 exited 0: original retained lifecycle 1/1 passed in 1.72s and closing-scope guard 1/1 in 0.03s; unrelated cases filtered.
- Host lint 34776, Windows all-target/all-feature lint 11970, typecheck 6614, formatting 24441, docs build/content/link check 60442 and managed check 15268 all exited 0.
- Windows eight-iteration execution remains pending native CI. Its implementation is present, but task 1.2 remains unchecked until actual execution. Historical OS5 phase and cause are still unknown; local passes and a future green run do not establish causal resolution. PR auto-merge stays off. This active record accompanies the diagnostic commit and must be finalized/archived before final merge.
