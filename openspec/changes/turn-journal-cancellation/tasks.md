## 1. Durable journal boundaries

- [x] 1.1 Add the narrow memory mutation that atomically appends messages in one namespace and upserts state through the existing receipt/reconciliation worker; verify validation, transaction rollback, lost acknowledgement, and exact committed rows against real Dolt
- [x] 1.2 Add the bounded state-backed journal model, exact request/ID lookup, transitions, and conservative possible-dispatch state; verify completed reuse, mismatch rejection, safe pre-dispatch resume, process restart, and full retained lifecycle history
- [x] 1.3 Replace turn admission and completion writes with the two atomic checkpoints while preserving the early user transcript and pending-publication rules; verify interrupted and completed transcript/session/topology/output states without duplicates

## 2. Explicit runtime cancellation

- [x] 2.1 Add one cancellation token and controlled run entrypoint, carry it through actor mailbox/semaphore/provider waits, and persist interruption after accepted writes settle; verify cancellation at each actor boundary releases the permit and permits a later turn
- [x] 2.2 Observe the same token around built-in/MCP tools, cognitive operations, peer requests, and outbound A2A without changing provider or ToolHost public traits; verify pre-dispatch exclusion, one observed ambiguous dispatch without replay, reconciled memory writes, and retained process cleanup
- [x] 2.3 Make turn and standalone dreaming cancellable across actor work, candidate writes, and promotion reconciliation; verify active history remains valid, accepted promotion is published exactly, and abandoned candidates are not deleted
- [x] 2.4 Freeze the existing-shape `TurnOutput` at the ended checkpoint before response publication and periodic dream maintenance; verify cancellation or dream failure after completion returns the stored byte-equivalent output and only broadcasts later dream activity

## 3. Application and shutdown integration

- [x] 3.1 Replace normal TUI job abort with signal-and-settle cancellation while retaining generation fencing and abnormal-exit cleanup; verify real PTY interruption and next-command usability, compose durable and typed completion-race authority at the shared UI boundary, and preserve plain/JSON run shapes
- [x] 3.2 Route inbound A2A message IDs through controlled turns and return bounded mismatch/uncertainty failures; verify duplicate completed requests reuse one answer without another provider/tool call
- [x] 3.3 Bound shutdown dreaming with an aggregate cancellation deadline and always attempt existing actor and ToolHost cleanup; verify dream timeout and cleanup failure are both reported without treating the deadline as process cleanup proof

## 4. Documentation and coordinated verification

- [x] 4.1 Document turn IDs, retry/uncertainty, interrupted-prompt retention, cancellation, answer/dream event boundaries, and shutdown limits in owning runtime, usage, protocol, and curated docs; verify the built site and content checks
- [ ] 4.2 After the archived checkpoint parent lands, modify its `Asynchronous terminal event scheduling` requirement in the existing chat-harness delta so normal cancellation signals and settles while abnormal cleanup signals when a token is retained and otherwise aborts; strict-validate the integrated delta before archive or merge
- [ ] 4.3 Run the focused memory, connector, runtime, TUI, format, lint, typecheck, and docs checks, then the single coordinated workspace coverage task; record exact exits and retain the 90 percent floor
- [ ] 4.4 Run native macOS/Linux and Windows real-Dolt, process-owner, A2A, and PTY cancellation fixtures; record platform-specific evidence and defer only genuinely unavailable native runs
