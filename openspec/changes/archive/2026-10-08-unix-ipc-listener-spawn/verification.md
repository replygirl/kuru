# Verification

## 1. Private listener ownership [critical]

- [x] 1.1 @regression (agent) hold the existing spawn lock while another native thread attempts binding, outgoing connection creation, or queued acceptance -> all three original regressions failed with the observed first event false (exit 101); corrected binding passes 1/1 and outgoing/accepted creation passes 2/2, with actual byte exchange after release
- [x] 1.2 @integration (agent) keep an actual owned child alive while dropping a private listener -> real cat roundtrip establishes the running child; exact stale endpoint returns ConnectionRefused within the existing five-second bound, retains its identity and adjacent bytes, then the child is explicitly reaped
- [x] 1.3 @integration (agent) run the affected platform suite and static checks on macOS -> final platform behavior passes 145/145; final platform lint, formatting and public docs checks pass. Independent pending waiters, stale-parent refusal and refused-spawn successor recovery pass. Fresh pinned local coverage is 5897/6210 (94.959742%); its unchanged 95% gate fails, so final coverage acceptance remains pending under 1.4 and the active enclosing goal
- [~] 1.4 @runtime (agent) execute the platform cases in native PR CI and exact-main CI -> defer: these external runs follow the local bug-fix source archive. Every native PR and exact-main check and every existing 95% gate remains required before merge and goal completion; actual results belong in the working document. No final coverage or native Windows execution pass is claimed from local behavior
