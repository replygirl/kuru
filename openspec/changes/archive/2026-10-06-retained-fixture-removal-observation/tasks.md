# Tasks

## 1. Owned removal observation

- [x] 1.1 Contextualize retained-directory handoff, pool acquisition and removal observations in server_lifecycle.rs; preserve all original lifecycle assertions.
- [x] 1.2 Add a native Windows causal held-root deletion-pending test with an actual denied observation and pending-before-release/absence-after-release assertions; retain the last uncertain error under the unchanged deadline.

## 2. Verification and delivery

- [x] 2.1 Run the original retained lifecycle case and closing guard through the owning task; run affected host/Windows compiler, lint, formatting, docs and managed checks. Record native Windows execution as CI pending locally.
- [x] 2.2 Obtain independent source review, record exact observed results and limitations, validate and archive normally with physical archive verification, then make a hooked conventional commit for Product delivery.

## Observed evidence

- Actual strict validation and apply both exited 0; all three returned context files were read before source edits. No active blockers were listed.
- Owning prepared memory test handle 80011 exited 0: original retained-supervisor lifecycle case 1/1 passed, 2.59s; all other cases filtered. Closing-scope guard selection 39009 exited 0, all unrelated cases filtered.
- Final host lint 17705, Windows all-target/all-feature lint 65551, formatting 80595, typecheck 2067, docs build/content/link checks 2689 and managed drift 68174 all exited 0. First combined static invocation 25346 forwarded task names to Clippy and failed before meaningful source checks; separate invocations replaced it. First host/Windows lint runs found only collapsible_if; the equivalent collapsed condition passed the final checks.
- Independent Architecture source review cleared the substantive frozen diff; the final change only collapses the nested deadline condition required by Clippy. Final source SHA256: c8243b60b1433c16093161e75fd39a4e1fed76ebe78149a5423915ccf108a05b.
- Native Windows held-root behavior has not run locally and must pass native CI. Compiler/lint success is not native behavioral proof. The new case requires an actual OS5 probe, actual Pending before exact handle release, then confirmed absence; it never maps access denied to absence. Existing 15s deadline and 25ms interval are unchanged.
- The historical bare OS5 operation remains unknown; no production reaper, lifecycle, process, configuration or dependency changes are made. No coverage threshold, deletion retry or deadline relaxation is introduced.
