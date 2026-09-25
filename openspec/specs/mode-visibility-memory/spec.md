# mode-visibility-memory Specification

## Purpose
Apply validated mode visibility and memory decisions to runtime context, peer
delivery, private storage and consolidation while preserving mandatory request
material, identity isolation and runtime-owned durable publication.

## Requirements

### Requirement: Effective visibility dispatch preserves mandatory material

The runtime SHALL obtain `VisibilityPolicy::context_sources` for every
provider request, including deliberate, speaking, consultation and dream
requests. It MUST validate the returned sources before any selected private or
public source is read. Every effective request SHALL retain exactly the current
explicit input and any required tool receipt, call chain and native
continuation material even when a policy omits older history or public
transcript. The runtime SHALL retain typed `ActorPhase` for dispatch,
accounting and events; policy-provided descriptive phase text MUST NOT replace
that typed phase. A relationship actor SHALL use only its own relationship
history and notes plus explicitly supplied member contributions.

#### Scenario: Omitted ambient context remains accounted

- **WHEN** a test-only visibility policy omits public transcript or own notes
- **THEN** the provider request and P7 itemized fit estimate omit the same
  optional material, while current input and receipts remain present and
  durable history is unchanged.

#### Scenario: Missing explicit input is refused

- **WHEN** a visibility policy omits `ExplicitInput` for an actor request
- **THEN** the runtime refuses before the private read or provider dispatch.

### Requirement: Visibility delivery decisions precede peer effects

The runtime SHALL require both the existing Peering decision and
`VisibilityPolicy::allows_delivery` before it emits a peer-delivery event or
queues peer mail. A rejected visibility decision SHALL produce the existing
truthful cognitive-tool refusal and SHALL NOT enqueue mail, invoke a provider
or weaken P6 permission enforcement.

#### Scenario: Delivery visibility denial

- **WHEN** a test-only visibility policy rejects an otherwise valid live peer
  edge
- **THEN** no peer event, mailbox entry or recipient provider request occurs.

### Requirement: Effective memory namespace and state dispatch

The runtime SHALL validate and consume `MemoryPolicy` identity, transcript and
state-key decisions before each corresponding actor spawn, private history or
note read/write, public transcript checkpoint/read, topology read/write, dream
candidate operation and dream undo. It MUST reject a namespace that crosses an
identity's private scope before the read or write. A selected profile SHALL
remain paired with its state through existing candidate, uncertain-write,
reconcile and publication handling. Built-in namespace strings and four-mode
behavior SHALL remain byte-equivalent.

#### Scenario: Distinct private namespace survives lifecycle boundaries

- **WHEN** a test-only memory policy selects a valid distinct suffix for one
  identity
- **THEN** its private notes and history use that namespace through reopen,
  candidate abandonment or promotion, retirement and undo without exposing
  another identity's material.

### Requirement: Runtime-owned validated consolidation plan

Before every dream, the runtime SHALL call `MemoryPolicy::consolidation_plan`
with the actual active parts and validate the result against that topology. The
plan participants MUST be a unique ordered subset of live active parts; its
prompt and descriptive phase MUST be nonempty; and its
`max_proposals_per_part` MUST NOT exceed two. The runtime SHALL consume the
returned participant order, prompt, phase and limit while retaining typed Dream
phase, P7 accounting, dream-only tools, candidate writes, promotion,
reconciliation, cancellation and compensating undo ownership.

#### Scenario: Restricted ordered dream plan

- **WHEN** a test-only memory policy selects a proper ordered subset of current
  active parts with a bounded proposal limit
- **THEN** only those parts receive dream requests, work is assembled and results
  associated in plan order under the existing parallel budget, and candidate
  promotion and abandonment retain their existing behavior.

#### Scenario: Invalid dream plan is refused

- **WHEN** a consolidation plan repeats, names an inactive part, exceeds two
  proposals per participant or has blank prompt or phase text
- **THEN** the runtime refuses before any dream provider request or candidate
  mutation.

### Requirement: Effective policies govern compaction source and summary continuity

Before reading a compaction source or a settled compaction summary, the runtime SHALL obtain and validate the selected mode's effective visibility and memory-policy decisions for that exact actor phase and namespace. Automatic and manual compaction MUST summarize only policy-admitted raw current-session history. Later ordinary requests MAY include current-session or cross-session cursor-selected context summaries only when policy admits that summary namespace. Policy admission MUST NOT convert producer-private reasoning summaries, including operation-attributed Compact sidecars, public transcript rows, notes or another actor's raw history into compaction source.

#### Scenario: Policy omits history or cross-session summaries
- **WHEN** a test policy omits an actor's raw history or cross-session summary continuity
- **THEN** compaction refuses or excludes that source before reading it, and later context contains neither omitted summaries nor a fallback namespace.

#### Scenario: Policy changes after snapshot
- **WHEN** effective policy or actor namespace changes after a source snapshot but before publication
- **THEN** revalidation prevents publication under stale authority and leaves the previous summary/cursor projection usable.
