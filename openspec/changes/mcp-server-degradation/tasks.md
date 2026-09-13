## 1. Owned stdio foundation

- [x] 1.1 Add one-shot stdin extraction to the Unix process-group owner and make Scanner reservation track the current destination buffer while preserving cumulative bounds; verify one-shot pipes and long cleared-buffer scans with focused platform/connector tests
- [x] 1.2 Replace stdio RPC child ownership with the private worker, continuous projected stderr drain, bounded diagnostic tail and common cancellation/timeout/close cleanup; verify real caller/runtime-loss, descendant-held-pipe, flood, framing and native ownership fixtures

## 2. Alias-local MCP state

- [x] 2.1 Add the minimal catalog/status values and serialize each client's complete discovery/call/close operations; verify healthy and failed aliases in both orders plus multipage, invalid, repeated and oversized catalog candidates
- [x] 2.2 Publish complete routes per alias, keep prior failed routes inactive, disable ambiguous calls without replay and recover only through explicit discovery; verify received-once disconnect/cancellation, zero disabled dispatch and healthy `isError` reuse
- [x] 2.3 Close host admission before bounded concurrent client cleanup; verify starting, active and idle shutdown races admit no new client and successful shutdown confirms every admitted cleanup

## 3. Runtime and user presentation

- [x] 3.1 Consume the catalog in runtime and direct CLI paths, preserving tool/result stdout while projecting fixed runtime metadata and human-only safe diagnostics; verify real CLI, provider-turn, event, prompt and reopened-memory observations
- [x] 3.2 Update protocol and curated tools documentation with alias degradation, explicit recovery, no replay, finite stderr filtering and process-authority limits; verify the built site and content checks contain the public contract without private notes

## 4. Coordinated verification

- [ ] 4.1 Run focused format, lint, typecheck and affected package tests while implementing, then run the single workspace coverage writer only when coordinated; record exact exits and preserve the 90% line floor without duplicate broad suites
- [ ] 4.2 Run native macOS/Linux and Windows MCP ownership and stderr fixtures through their owning tasks; record observed evidence per platform and leave genuinely unavailable native execution explicitly deferred

Pending: the coordinated workspace coverage writer and native Linux/Windows execution have not run. Local macOS focused checks are recorded in `verification.md`.
