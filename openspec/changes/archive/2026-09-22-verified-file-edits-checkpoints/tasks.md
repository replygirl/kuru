## 1. Contract and private store

- [x] 1.1 Define bounded edit/checkpoint request and receipt types with explicit operation identity, state and capacity validation; verify malformed, oversized and changed-ID unit cases.
- [x] 1.2 Implement project-bound private checkpoint storage, lock, durable prepared/applied/uncertain records and explicit selected pruning; verify real file identity, crash/reopen and full-capacity fixtures.

## 2. Checked native file mutation

- [x] 2.1 Implement unique ordered exact-context `file_edit` hunk selection and shared checked publish/delete path for all three native file mutations; verify real multi-hunk, stale target, ambiguous context and no partial effect fixtures.
- [x] 2.2 Bind admitted session/turn/tool-call IDs to receipts, preserve existing permission and nested-instruction replan order, and reconcile exact retries without blind replay; verify canceled and accepted-lost-reply interactions through runtime/ToolHost fixtures.
- [x] 2.3 Implement selected undo with normal target authority and expected-post-state checks, including explicit uncertain-delete refusal; verify restart, user edit conflict and idempotent undo fixtures.

## 3. User surface and documentation

- [x] 3.1 Add bounded checkpoint inspect/undo/prune CLI and working TUI commands/notices only after the backend is active; verify real PTY and headless command behavior.
- [x] 3.2 Document private storage location, fixed bounds, explicit pruning, uncertainty and file-vs-dream undo boundary; verify docs build/content checks.

## 4. Integration and closure

- [x] 4.1 Run the verification ledger's local connector/runtime/TUI/native integration probes, record observed results, and leave exact-head hosted OS checks as explicit post-commit delivery gates.
- [x] 4.2 Run owning format/lint/typecheck/tests, strict Cospec validate/apply, managed drift and diff checks, obtain independent source review, then archive the completed change before final commit.
