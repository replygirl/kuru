## Context

The accepted P7 decision record is `tmp/roadmap/phase1-accounting-contract.md`. Current `MemoryStore` pins views to Dolt branches and serializes writes with `Shared.write`; dreams promote candidates against the live main revision. P5's connector collector offers fallible ordered stream events, but native Responses may substitute actor-local `Pending.input/output` for visible history when current tool receipts complete a continuation. P2 already supplies `ModelMetadata::resolved_context_window` and `assumed_context_window_tokens` with provenance. The design below fixes implementation seams without changing those contracts.

## Goals / Non-Goals

**Goals:** One durable provider-invocation fact source, a fit decision at the actual connector send boundary, and a small presentation projection that can say what is known, assumed, omitted or incomplete.

**Non-Goals:** A general writable Dolt branch API, a tokenizer accuracy claim, provider pricing fetch, Phase 2 compaction/reasoning persistence, event-sourcing changes, a shared pool context window, or new permission/mode policy.

## Decisions

### Use a permanent memory-owned operational branch

`kuru-memory` creates a reserved `kuru_usage_v1` branch at writable open, validates only its ledger-owned state/receipt schema, and exposes `UsageLedger` rather than a mutable `MemoryView`. Each admission/observation/settlement is a short receipted transaction under the existing writer serialization. Reconciliation precedes a later mutation after an uncertain reply. The branch is never promoted or rebased. A main-branch ledger was rejected because its commits could invalidate an open dream candidate; candidate-local usage was rejected because abandoned work would disappear. Existing `memory export` remains a live memory snapshot and explicitly documents exclusion of the operational usage ledger; no export schema or sidecar is added.

### Identify each invocation before dispatch and fold on read

Runtime creates a bounded opaque invocation key from session ID, admitted turn or durable dream-attempt ID, phase, actor and ordinal. The ordinal advances for every actual provider attempt, including repeated consultations or speaking rounds. The actor carries that identity in `Work`; `admit` durably inserts it before calling the provider. The ledger accepts sequence-numbered usage observations, terminal presence and final outcome on the same record. Identical replays are idempotent; conflicting replay fails. `/cost` folds records, not a second mutable total. A new-session marker distinguishes pre-ledger resumed sessions even when earlier turns are zero. One row freezes route, model and price schedule/basis/source; unknown and partial components remain `Option`, with input/output totals separate from cached/reasoning subsets. A standalone total cache was rejected because uncertain writes and retries could double count it.

### Meter the effective request at the connector boundary

Core defines bounded `ContextBudget`, `ContextSourceSize`, `ContextEstimate`/`ContextTooLarge` data without accepting private bytes. Runtime assembles source-labelled mandatory/optional units and selected model window plus output reserve; this includes public conversation embedded in instructions and notes appended to instructions, not only message rows. It may remove whole older optional rows and retry preflight under the same admitted invocation only before HTTP. The connector constructs `input_items` and the final wire body under the actor's pending-continuation lock, calls the existing fallible stream sink with non-content `ContextMeasured` after a fit check, and checks the same final body before HTTP. A direct/legacy request without a supplied budget receives the documented built-in assumed window, not an unchecked bypass. `ContextTooLarge` identifies whether native continuation makes the call/receipt chain mandatory. Pending input/output, current matching receipts, instructions and tool schemas are mandatory in native continuation rounds. The connector does not hand encrypted pending values to runtime or allow a runtime-visible-message estimate to override its actual body. This seam is separate from the existing 2 MiB transport-byte cap. Pure runtime sizing was rejected because it cannot see the selected native pending body.

### Settle observations through P5's fallible stream

The actor's stream observer assigns local monotonic usage sequence numbers, awaits `UsageLedger::observe`, and sees original `Completed.usage` before P5's collector fills missing fields from earlier observations. The actor settles success only after terminal usage durability; on failure/cancellation it settles the observed partial record and leaves absent values unknown. An accounting write error stops collection and surfaces; it cannot silently allow another unrecorded provider dispatch. The provider stream itself never writes Dolt or receives a memory handle. Holding a candidate/write mutex across HTTP was rejected because it would block unrelated memory progress and risk dream promotion.

### Present provenance and actual omissions

`SessionUsage` returns known sums, per-component completeness, historical marker status, frozen price estimates and their basis. `/cost` renders subscription cost only as an API-equivalent estimate, never a bill/quota. The runtime's context projection reports which source and exactly how many stored rows were omitted from this request, without deleting them. Status identifies the selected facing request when known, otherwise the active request, and labels the configured/built-in assumption for unknown windows. `context_output_reserve_tokens` is the only P7 config addition; absent override uses a bounded model-appropriate reserve from P2 metadata or a documented conservative default. An assumed window alone never blocks model selection.

## Operational surface

`/cost` and context status are in the existing attached TUI; scripted output retains its current stdout contract. There is no new listener, bind address, container or runner topology, required secret, or connection pool. The permanent ledger uses the same owned, pinned full-Dolt binary for each supported architecture and the existing project writer lease/supervisor; it does not download another engine or open an independent server. Native provider routes retain their existing request limits and configured credentials.

## Integration contract

The connector owns native Responses request-body construction and private `Pending` by actor; the runtime supplies source labels/window/reserve and receives bounded sizes plus typed fit refusal, never opaque continuation. Provider stream usage observations are local-sequence records, distinct from remote event IDs and from tool call IDs. The memory package owns the reserved Dolt branch, its state/operation-receipt schema and versioned invocation key; no filesystem mount, HTTP route or general SQL handle is added. P5 terminal `Completed.usage` is observed before collector fallback fills missing fields, so absent terminal components remain distinguishable from progress observations. Test fixtures cover the direct API-key and subscription native routes, multi-call continuation, partial usage and real-Dolt receipt reconciliation.

## Risks / Trade-offs

- **[Operational branch drift]** → Version and validate only its ledger-owned state/receipt contract at writable open; fail closed for new inference if it cannot be used, without rebasing to main.
- **[Provider report lost before durability]** → Persist admission before network, await every observed write, and mark interrupted/admitted records incomplete after reopen; external provider use and a local crash cannot be atomic.
- **[Native continuation exceeds visible estimate]** → Preflight the exact selected body under pending ownership and refuse mandatory overflow before HTTP; never slice opaque continuation or matching receipts.
- **[Approximate tokens or prices look exact]** → Label token fit as estimated, preserve source/window provenance and missing components, freeze historical price terms and distinguish subscription API-equivalent estimates.
- **[Shared-file integration with P6]** → Fast-forward its reviewed runtime/UI parent before P7 source edits; P6 is not an accounting behavior prerequisite.
