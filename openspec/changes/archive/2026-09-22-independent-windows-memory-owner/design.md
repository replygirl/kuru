## Context

`NativeSpawnSpec` offers `OwnedJob` and `TrustedSupervisor`. The Windows service starter chose `TrustedSupervisor`, so Kuru did not create a child Job, but `CreateProcessW` still inherits any Job already containing the starter. If that Job has kill-on-close, closing the starter's Job kills the supposedly independent memory service. Windows permits breakaway only when the containing Job allows it; silently starting inside a denying Job would violate the archived owner lifetime requirement.

## Goals / Non-Goals

**Goals:** Make Windows service lifetime independent of a starter's permitted Job, fail explicitly when the OS forbids that lifetime, and prove both paths natively.

**Non-Goals:** Change the existing Dolt supervisor's or browser's launch semantics, escape arbitrary OS containment, add a daemon installer, or alter the storage protocol.

## Decisions

Add a narrow `IndependentService` lifetime to the safe Windows platform process API. It uses the existing trusted child-handle behavior, omits an owned Job, and adds `CREATE_BREAKAWAY_FROM_JOB` to the synchronous `CreateProcessW` call. Windows ignores that flag when no Job contains the starter and rejects creation when the Job disallows breakaway. Kuru returns that rejection with an explicit independent-lifetime diagnostic, without retrying as `TrustedSupervisor`. Existing `OwnedJob` and `TrustedSupervisor` retain their flags and cleanup behavior.

Use the platform's test-support process fixture to create a starter under Jobs that respectively allow and deny breakaway. In the allowed case, release the starter's kill-on-close Job after it exits, then prove a surviving service connection can still commit and read against the same generation. In the denied case, prove no service endpoint or owner is published and the error identifies the containment limit. Retained process handles and checked endpoint/lock records, not a PID probe, establish the observation.

## Operational surface

No address, secret, binary format or installation mode changes. Ordinary Windows shells without a containing Job are unaffected. A Windows launcher or CI runner that contains Kuru in a Job must permit explicit breakaway for an independently living service; otherwise Kuru fails the service start with a diagnostic instead of claiming that a contained child can survive its starter. Hosted Windows native CI must run the allow and deny fixtures; cross-compilation is not behavioral evidence.

## Integration contract

`kuru-platform` owns the `CreateProcessW` flag and test-only Job setup. `kuru-memory` selects `IndependentService` only for its private Windows owner process and continues to hold/observe the returned process handle. The service still owns Dolt through the existing supervisor protocol. No consumer outside the memory service changes lifetime mode.

## Risks / Trade-offs

- **[Containing Job denies breakaway]** → Fail before publication with a precise startup error; no fallback can meet the independent-lifetime contract.
- **[Unexpected child cleanup]** → Keep the retained child observer and owner reaping paths; native tests cover starter exit and endpoint retirement.
- **[Process API regression]** → Existing native `OwnedJob` and `TrustedSupervisor` fixtures remain unchanged and run alongside new allow/deny cases.
