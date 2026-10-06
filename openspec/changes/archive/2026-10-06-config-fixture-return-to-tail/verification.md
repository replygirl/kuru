# Verification

## 1. Configuration inspection after historical reading [critical]

- [x] 1.1 @regression (agent) run the real synchronized configuration PTY fixture before and after explicit follow-tail navigation -> exact-644 Ubuntu CI job 112210407020 and macOS job 112210406537 failed at cli.rs:3314 while the completed screen retained Reading mode; corrected owning task 64548 exited 0 on macOS, selected real PTY case 1/1 passed in 6.27s with every original captured snapshot, redaction and current Freudian selection assertion unchanged. No extra local red run or broad suite was executed.
- [x] 1.2 @e2e (agent) observe completed PageDown frames returning from historical captured rows before later commands -> task 64548 passed the explicit completed-frame navigation loop until the history-return marker cleared, then both fresh Freudian configuration projections passed within unchanged READY_TIMEOUT deadlines.
- [x] 1.3 @integration (agent) run the existing store-opening test closing-scope guard alongside the selected fixture -> task 64548 passed the existing guard 1/1 in 0.03s; package-owned bundle preparation and prefetch completed, all other behavioral cases were filtered out.

## 2. Required scoped checks

- [x] 2.1 @integration (agent) run app host and Windows lint, all-target typecheck, format, docs, managed and strict checks -> owning host lint 19693, Windows-target lint 45360, app all-target/all-feature typecheck 83004, Rust format 8828, docs build/content/link/lint/format 50972 and managed drift check exited 0. Strict validation 566186 exited 0 before apply. Root independently reviewed the exact 21-line fixture diff and cleared scope. Remote PR238 and exact-main CI remain Product-owned and pending; Windows static compilation is not a native PTY runtime claim.

## Observed process limits

Actual strict validation 566186 and apply 31d6f9 exited 0 before implementation;
all five returned context files were read. Initial strict validation requested
the small operational-surface design and was corrected before apply. The apply
output warned about absent delta specs despite strict validation passing; this
fixture changes no specified behavior, so archive explicitly skips specs.

The first attempted combined static command forwarded task names to Rustfmt and
exited 1 before a valid check (`file does not exist`, handle 94299). Separate
owning commands corrected that invocation: Rust format 8828, docs build/content
and formatting 50972, managed drift check, and app typecheck 83004 exited 0.
This invocation error is not a failing behavioral test.

Windows lint exited 0 with an advisory shared-cache churn warning naming a
missing historical `windows_shell.rs` compiler input. No source or cache repair
was needed or performed. All local handles are terminal; native preparation and
behavior ownership were released after the selected task.
