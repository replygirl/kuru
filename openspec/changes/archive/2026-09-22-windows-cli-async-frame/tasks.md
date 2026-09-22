## 1. Bound the binary dispatch frame

- [x] 1.1 Extract the unchanged async branch dispatch from the Tokio main function and heap-pin that one future at entry; verify argument parsing, internal helper routing, error propagation, and cleanup ordering remain identical in source review.
- [x] 1.2 Exclude every transient stack diagnostic probe from this main-based fix; verify the production diff touches only the binary entry and carries no stack-size override or new runtime task.

## 2. Regression and delivery evidence

- [x] 2.1 Run the owning TUI format, lint, typecheck, and focused local CLI/PTY tests with the verified native bundle, then strict Cospec validate/apply and diff checks; record actual outcomes.
- [x] 2.2 Name the existing Windows application CLI and terminal tests as the regression, record the pre-fix stack-overflow outcomes on #54/#56, and hand off the exact fix-head and dependent-integration native checks for post-push CI. Report their observed results in the PR; a clean current-main run alone does not prove the dependent fault is cured.
- [x] 2.3 Complete the Cospec verification record and archive before the branch commit; let the normal pre-push hook and native CI establish release evidence, with any post-commit results recorded in the PR/verification follow-up.
