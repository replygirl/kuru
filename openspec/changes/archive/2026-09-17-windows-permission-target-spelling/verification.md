## 1. Windows file-tool permission admission [critical]

- [~] 1.1 @regression (agent) run the existing automatic-permission CLI fixture on native Windows before and after the correction -> defer: old PR37 head 9af7602, run 35183200733/job 105079614737 failed at `cli.rs:1372` on an untyped refusal; corrected native Windows CI awaits stack restack and push
- [~] 1.2 @integration (agent) exercise ordinary and verbatim Windows root spellings with checked file tools -> defer: Windows-only retained-root regression added; this macOS host cannot execute it, and cross-target checking stops in `aws-lc-sys` because Windows SDK headers are absent
- [~] 1.3 @e2e (agent) run the native Windows CLI permission flow -> defer: corrected native Windows app job awaits stack restack; focused macOS CLI permission fixture passed with no file effect

## 2. Scope and local checks

- [x] 2.1 @unit (agent) run focused connector and CLI permission tests on macOS -> `//packages/kuru-connectors:test -- central_permission_gate_denies_effects_and_explicit_allow_overrides_legacy_false` passed 1/1; `//apps/kuru-tui:test -- automatic_permission_claim_is_reviewed_but_trust_does_not_grant_a_call` passed 1/1 with prepared Dolt supervisor
- [x] 2.2 @integration (agent) run scoped connector/app typecheck, lint, Rust format, and strict Cospec validation -> both package typechecks, both package lints, `//:format:rust`, and strict Cospec validate exited 0 on macOS
- [~] 2.3 @eval (agent) compare allow and unattended ask decisions for the same checked file target -> defer: Windows-only regression makes both decisions observable on the retained ordinary/verbatim root; native run awaits corrected CI
