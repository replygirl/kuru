# Design

## Context

The owner now requires a real session claim for authenticated private mutations. Old managed fixtures assumed attachment alone granted authority; runtime constructors now derive a separate bound view, so observing the input attachment misses actual barriers. Checked recovery deliberately retains a probe while installing an identified successor attachment. Local close acknowledges socket disposal, while owner-side claim disposal follows actual EOF independently.

## Decisions

Use existing bind/select APIs with the captured catalog, retain the driver during mutation and recovery, and await ordinary close before root release. Fresh fixture catalog creation is allowed only for newly authored fixture sessions; migrated sessions retain their original catalog, journal and private identities. Runtime actor work and causal observers use the actual bound memory view. Same-driver concurrent fixture work retains its original claim; separate older sessions get separate claims. Recovery fixtures assert every probe/identified connection/release event. Presence release is proved by actual owner inventory within the existing bound, rather than a scheduling delay.

Restore the CLI's checked data/locks preflight and existing privacy error context before runtime activation. Reuse the existing verified project-lock opener with exclusive maintenance and shared runtime-startup modes. Acquire shared ownership before diagnostics or memory activation for ordinary runtime opens and retain it through exact session admission and actual memory cleanup. This permits independent ordinary sessions while refusing exclusive maintenance before SQL initialization. Legacy activation keeps its existing exclusive lease until complete; release that lease and acquire the same checked shared lock before continuation/resume/admission/provider effects. No effect occurs between release and shared acquisition, and any refusal follows the existing already-owned memory cleanup. Keep maintenance contention distinct from session busy refusal.

Two existing concurrent-write fixtures need an independently owned exchange while the actor's reply is held. A test-support-only helper requires a settled writable main view, connects through its unchanged authenticated factory, and constructs the existing RemoteSession::new_view. It retains the same actual driver/client/proof but separate mutation receipts; it mints no claim, transfers no pending receipt and adds no production API or RPC. Construct before the held operation, keep the Harness driver alive, and explicitly close both exchange owners. Local parity clones the existing view.

## Operational surface

No bind address, service connection limit, binary version, architecture, secret or workflow changes. Tests remain native isolated temporary projects/stores with package-owned prepared supervisors and the verified private Dolt cache. CLI errors disclose checked path/privacy context; no credential stores or provider calls are needed. Native Windows and full coverage are subsequent CI evidence.

## Risks / Trade-offs

Additional real driver connections affect observed connection counts, so fixtures must preserve complete EOF/Joined and quiescence checks. No missing receipt, empty successor map, elapsed delay or local close reply is used as durable mutation or owner-release proof. Existing budgets and all safety assertions remain unchanged.
