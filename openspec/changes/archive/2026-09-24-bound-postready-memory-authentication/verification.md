## 1. One bounded owned startup [critical]

- [x] 1.1 @regression (agent) hold the observed real-Dolt initial SQLx callback for 2.3 seconds inside acquisition -> corrected open authenticates and verifies the exact owner without retry; focused native test passed 1/1. The controlled callback delay does not recreate the Windows pre-callback stall.
- [x] 1.2 @integration (agent) hold the observed callback beyond the normal 30-second startup allowance plus existing two-second transport margin -> open fails with typed SQLx `PoolTimedOut` at the shared deadline, publishes no usable endpoint, releases the Server/supervisor lifecycle lease after owner reap, and permits a fresh checked open; focused native test passed 1/1. This does not exercise Store's separate outer startup lock.
- [x] 1.3 @manual (agent) inspect readiness, initial acquisition, identity verification and ordinary pool paths -> one deadline bounds startup while ordinary and attached pools retain a fixed two-second acquisition policy; memory all-target Clippy and independent source review passed.

## 2. Owning and hosted checks

- [x] 2.1 @integration (agent) run package-owned Dolt prefetch, startup-budget, remote checkpoint lost-reply, and prior failing provider-free undo fixtures -> prefetch passed and the three focused native tests passed 1/1 each.
- [x] 2.2 @runtime (agent) run memory all-target lint, format, docs and strict Cospec validation -> all pass; normal commit/push hooks are recorded at checkpoint.
- [~] 2.3 @e2e (agent) validate the published exact P29 head across native Windows, macOS and Ubuntu jobs and combined coverage before merge -> defer: final-head publication and hosted jobs follow this local archive; the earlier Windows failure on head `d4462993` predates this correction and is not final-head evidence. Merge remains gated on full native and combined coverage results.
