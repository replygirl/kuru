# Verification

## 1. Coverage and advisory task execution [critical]

- [x] 1.1 @integration (agent) run the focused delivery task-launcher regressions on macOS -> final test passes for platform, archive and workspace launchers: exact managed tool selection and wrong-version refusal remain enforced. Test-only original configuration failed with exit 101 before correction
- [x] 1.2 @integration (agent) run formatting and affected host/Windows-target lint -> Rust/TOML formatting and final delivery host and Windows-target lint pass without tool or lockfile updates
- [~] 1.3 @runtime (agent) run native Windows PR CI through the corrected package tasks and launcher regression -> defer: actual native Windows execution follows the completed local source archive in PR #266; stock PowerShell shell/module-path reconstruction, native exit 7 and missing-program refusal are exercised by the fixture. Every PR and exact-main check, including every existing 95% gate, remains mandatory before merge and delivery. Actual results belong in the working document
