# Architecture

Kuru's native unit is a persistent part in a pool of equal actors. A part owns
its history and can send messages to any active peer. There is no LLM that
owns, directs or summarizes every other model call. The runtime decides when
mailboxes run and enforces resource limits mechanically.

```mermaid
flowchart LR
    U[Chat / CLI] --> R[Bounded runtime]
    R --> A[Part A]
    R --> B[Part B]
    R --> C[Part C]
    A <--> B
    B <--> C
    C <--> A
    A --> MA[(Private A)]
    B --> MB[(Private B)]
    C --> MC[(Private C)]
    A <--> G[Temporary relationship]
    B <--> G
    G --> MG[(Relationship history)]
    R --> P[Provider interface]
    R --> T[Tool host / MCP / A2A]
```

The diagram shows data ownership and routing. The runtime invokes the provider
for each selected actor; actors are logical Tokio tasks backed by model calls,
not separate pretrained models or proof of distinct subjective experience.

## Frameworks

Built-in frameworks establish named roles, instructions and initial membership.
IFS includes Self, managers, firefighters and exiles. The polyvagal mode uses
modeled autonomic states; Freud uses id, ego and superego; Jung uses its own
archetypal roles. These are software interpretations rather than scientific
validation of a framework. A role's instructions influence what it notices and
proposes, without granting it authority over other actors.

Stable IDs identify parts. Switching a mode selects a different topology;
provider/model/effort are independent configuration choices. Adding a framework
belongs in `kuru-core`, keeping provider code unchanged.

Core defines each built-in mode as a validated reference profile with six
separable, pure decision components: roles (seeds and coverage), peering
(eligible edges and relationships), flow (recipients and contribution envelopes),
facing (speaker choice), visibility (admitted context sources), and memory
(namespace keys and dream consolidation plan). These components return decisions;
the runtime still checks live identities and isolation, enforces budgets and tool
permissions, owns candidate memory, and performs every provider or tool call.
The current engine, actor, and dream loops retain their existing dispatch logic.
Routing those loops through the profile is later work; this contract alone does
not add a mode or a user control.

Speaker selection preserves stronger runtime evidence before consulting a
framework's authored order. An explicit caller target wins first, followed by an
existing active runtime focus. Otherwise the greatest activation narrows the
eligible candidates; if the previous completed speaker remains in an equal
maximum, that continuity wins. Only a remaining tie consults the built-in order.
The first eligible authored identity is Self for IFS, Connection for polyvagal,
Desire for Freudian and Continuity for Jungian, with missing or ineligible
identities skipped in favor of the next authored member. The selection event
records `mode-authored-order`. If none of the tied eligible candidates belongs to
the built-in authored list, including a dream-only tie, stable ID order decides
and the event records `stable-id-order`. These are deterministic tie-break
reasons for the current turn; they grant no identity authority or supervisory
role over its peers.

## Peer and relationship state

Every actor receives its own private history and the context explicitly made
available by the session. Other actors' private histories are not combined in
its prompt. The shared user conversation is a separate namespace. Model
reports of feelings or intentions are recorded as modeled state.

Protection, polarization and alliance relationships contain two to four
unique active members. Canonical relationship IDs are independent of member
ordering. A relationship can become the speaking identity and retain its own
history after it stops speaking. Nonmembers have no implicit access to that
history. Being a member of a relationship does not merge the members' private
memories.

Dolt persists namespaces in versioned transactions. Project identity derives from a
canonical path hash; state lives in a user data location outside tool roots.
Jungian collective memory is project-scoped in v1. A future explicit policy can
add cross-project scope without treating all user projects as one memory.

Each admitted turn writes one user transcript row together with a session-scoped
journal entry. A local CLI or TUI admission also replaces the session's single
last-submission reference with that turn's exact ID, prompt and target. Before
actor work, the runtime records that external dispatch is possible. A completed
answer, its assistant row and the matching session and topology state commit
together. An interrupted outcome instead commits one fixed internal-role
transcript marker; that role is visible as a Kuru marker but excluded from model
conversation context.

Turn-journal rows are durable no-expiry safety and idempotency history. Their
storage grows in proportion to admitted turns, and completed `TurnOutput` data is
retained indefinitely for exact retry. This intentionally duplicates some data
from the assistant transcript even though response events omit the answer body.
The journal has no aggregate cap, TTL or automatic deletion. It is separate from
the bounded operational diagnostics ring.

## Dreaming

Dreaming solicits bounded proposals from parts using their isolated context.
The runtime validates proposed additions and retirements, enforces `max_parts`
and preserves at least one active part for each role. Retired parts are archived;
their memories are retained. Saved prior topology permits undo. Explicit,
periodic and session-end triggers share the same validation path.

Each dream runs on a candidate branch, including its actor histories, summaries,
tool receipts and proposed topology. Promotion requires the recorded live base;
an interruption before promotion or a stale candidate leaves active memory intact.
An accepted promotion may finish after cancellation; the runtime reconciles its
durable result before further work. Undo adds a new
revision restoring membership while retaining later conversations and choices.
`kuru-memory` owns branch-pinned SQL views, verified engine installation and the
authenticated local sidecar. A lifetime supervisor reaps the sidecar on exit or
writer crash. No database process becomes a cognitive supervisor.

Dreaming is a consolidation and topology-update mechanism. It does not run an
unbounded background loop or imply biological sleep. Its provider calls count
toward the operational cost of a session.

## Context and usage accounting

Runtime assigns each provider invocation an identity and admits it durably before
inference. The actor awaits usage writes through the fallible provider stream and
records success, failure or cancellation separately from reported token counts.
Missing reports remain incomplete. The memory package keeps these operational
records on a permanent project-owned Dolt branch, outside live and candidate
history. Usage commits cannot invalidate a dream's promotion base or disappear
with an abandoned candidate. Session totals are folded from invocation records;
there is no independently mutable total to replay twice.

Context assembly inventories optional history separately from mandatory
instructions, current input and tool receipts. It can omit whole older rows
without changing storage. Connectors check the actual serialized request,
including actor-private native continuation, immediately before dispatch.
Only non-content measurements leave that boundary. The status and `/cost`
projections distinguish estimates, known reports and incomplete history.

## Message representation

Core messages carry a role and ordered content blocks. Text, tool uses and tool
results retain their order and call identities; completions carry usage and
stop metadata separately. Text displayed by the current CLI and TUI is a
projection of that content, not a second mutable copy. The final `TurnOutput`
remains authoritative, including when a completed turn is retried.

Reasoning-summary, image and cache-boundary types establish a shared data
contract. They do not enable image input, visible token streaming, new reasoning
history capture or prompt-cache optimization. Native encrypted reasoning used
to continue a tool round remains transient and scoped to its actor. It is not
written into conversation memory or exposed by inspection.

## Extension boundaries

| Boundary | Adding a capability |
| --- | --- |
| `Config` and validation | Add a typed option, default, validation and layered-config test |
| `Framework` | Add roles/instructions and topology invariants |
| `Provider` | Implement dynamic `models` and normalized `complete` |
| `ToolHost` | Add a schema and bounded execution handler with permission checks |
| Protocol adapters | Translate MCP/A2A messages at the boundary |
| Runtime envelopes | Extend peer actions and validate them before mutating state |
| TUI/CLI | Expose options while keeping cognitive policy in the runtime |

A provider only performs inference. The connector package owns native OpenAI
authentication, Kuru's private token store and direct HTTP transports. The
`codex` provider uses ChatGPT subscription access; `responses` uses an explicitly
selected API-key route. Neither launches an external Codex harness. Kuru owns
actor context, memory and tools. General-purpose worker subtrees are outside
the initial architecture. A2A ingress/egress provides the
path to external peers, with explicit configured endpoints rather than remote
autodiscovery or implicit trust.

## Bounds and permissions

Concurrency, peer rounds and tool-call budgets bound cyclic conversation.
Cancellation is an explicit signal through actor, provider, tool, A2A and dream
waits. Accepted memory writes still settle or reconcile, and shell or MCP owners
keep responsibility for their process cleanup. After workspace trust, core
permission rules select allow, ask or deny. The connector service checks the
validated invocation at native/MCP execution and before runtime outbound A2A.
Explicit deny always wins; an unresolved ask without a foreground approval
channel returns a typed refusal. The app owns private persistent grant files;
session grants live only in the running session, and once decisions remain
bound to one invocation. No approval channel is inherited by background work.

File tools retain root containment and protected-path checks regardless of
grants. Shell execution is ordinary process authority. A working directory is
not a security sandbox. MCP servers and configured external endpoints can have
their own authority beyond Kuru's built-in file tools. See
[tool permissions](configuration.md#tool-permissions) for rule precedence and
approval scopes.
