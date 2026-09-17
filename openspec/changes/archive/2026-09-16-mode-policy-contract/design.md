## Context

`Framework::builtin` currently owns the four seed tables; runtime `engine.rs`, `actor.rs` and `dream.rs` make the other five policy decisions directly. The P3 contract precedes P8/P9 loop extraction. Existing serialized `Mode`, `Part`, topology and namespace strings are durable compatibility boundaries.

## Goals / Non-Goals

**Goals:** Pure, independently callable operations with exact built-in parity; a narrow validation point and pre-extraction golden evidence.

**Non-Goals:** A declarative/user mode format, generalized policy composition, namespace capability tokens, new topology, new focus UI, supervisor role or loop replacement.

## Decisions

1. **Six small component interfaces, one built-in profile.** `ModeProfile` contains role, peering, flow, facing, visibility and memory components. Each consumes owned or borrowed typed snapshots and returns values only. Closed `Mode -> builtin profile` resolution is the sole production constructor. Test-only profiles may substitute an axis. Rejected alternative: one `Framework` callback over mutable `Harness`; it would merge concerns and transfer executor authority into policy code.
2. **Roles keep the current seed formula.** Move the existing authored tuples into a role component and keep `Framework::builtin` as a compatibility adapter. Generate IDs with the unchanged v5 UUID input, call the same canonical instruction constructor, and compare complete byte strings with golden fixtures. Rejected alternative: re-key seeds or change persisted `Mode` serialization during extraction.
3. **Policies return plans within runtime-known source types.** Peering decides candidate edges/members; flow returns ordered recipients and contribution envelopes; facing returns `{id, reason}`; visibility chooses from own-history, own-notes, public-transcript and explicit-input sources; memory returns current exact namespace keys and a candidate-only consolidation selection. The runtime validates active identities, membership, source scope, paths and role coverage before applying any future policy result. Rejected alternative: accepting arbitrary private-namespace strings as an access grant or handing policy a storage handle.
4. **Safety checks remain outside mode customization.** Tool permission, budgets, one-hop execution, candidate promotion/reconcile and compensating undo stay runtime/memory rules. Profile validation covers declared seed uniqueness/order/coverage and permitted source/path shapes; live topology and each produced result are validated again by the runtime. Rejected alternative: treating profile validation as a one-time exemption from output checking.
5. **Baseline before dispatch extraction.** Keep P3 runtime scenarios on current loops and capture normalized provider request/turn data for every mode. P8/P9 reuse those fixtures against extracted calls and may refresh only intentional earlier P1/P4/P7 representation changes. Rejected alternative: proving parity only with unit tests of the new policy functions.

## Risks / Trade-offs

- [An abstract contract may become nominal before P8/P9.] → Require callable pure operations and test-only alternative decisions now, then require P8/P9 to demonstrate every axis affects the real runtime.
- [A generic profile could accidentally grant cross-private access.] → Restrict source variants and validate every returned identity/path at use, with negative cases.
- [Golden assertions could freeze incidental identifiers.] → Normalize only generated session/turn IDs and timing, while retaining seed IDs, ordered requests and semantic fields.
