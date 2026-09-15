## 1. Process-wide retained-shell admission [critical]

- [x] 1.1 @regression (agent) retain real Unix-owned workers across independent registries with a small injected shared cap -> `mise run //packages/kuru-connectors:test -- unix_shell`: 16 focused tests passed; overload was rejected before its command created a file.
- [x] 1.2 @regression (agent) drop a caller and registry while its real owner has interrupted cleanup, then permit confirmation -> `mise run //packages/kuru-connectors:test -- unix_shell`: retained slot stayed occupied until confirmed cleanup and one later request completed.
- [x] 1.3 @unit (agent) inject thread, runtime, and pre-spawn failures after admission -> `mise run //packages/kuru-connectors:test -- unix_shell`: failures left the injected one-slot admission at zero with no child.

## 2. Retained cleanup pacing

- [x] 2.1 @regression (agent) hold real-owned cleanup interrupted across small injected backoff intervals -> `mise run //packages/kuru-connectors:test -- unix_shell`: retained attempts were at least 15 ms then 35 ms apart for injected 20 ms then 40 ms bounds, with no transition before release.
- [x] 2.2 @unit (agent) run existing Unix shell ownership tests -> `mise run //packages/kuru-connectors:test -- unix_shell`: 16 focused Unix-shell tests passed, including cancellation, caller/runtime loss, completion ordering, and interrupted no-stale-PID cleanup.
