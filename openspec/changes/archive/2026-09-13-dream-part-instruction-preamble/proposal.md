## Why

Builtin framework peers persist an equal-peer instruction preamble before each
authored tendency, but an accepted dream `Add` proposal currently persists its
raw instruction unchanged. A newly dream-added peer can therefore miss the
same durable equal-peer framing while otherwise entering the normal candidate
and promotion path.

The correction must not reinterpret authored content or repair existing
history. The existing 8,192-byte input bound applies to the incoming proposal
before any normalization or wrapping, so a valid near-limit tendency may have
a larger persisted instruction and cannot necessarily be submitted again as a
new `Add` proposal.

## What Changes

- Define one core-owned structural constructor for a newly added peer's
  instruction. It enforces the existing raw incoming 8,192-byte bound before
  normalization; its canonical prefix is exactly `PEER_INSTRUCTION + "\n\nYour
  tendency: "`; it removes only repeated exact copies of that prefix at the
  start of a valid incoming instruction and preserves the remaining authored
  tail byte-for-byte.
- Retain runtime `Add` validation for name, role, capacity, and topology.
  Reject blank or prefix-only input after the
  repeated leading prefixes are removed. Canonicalization is idempotent only
  for inputs that remain within that incoming bound; it does not trim an
  oversized wrapped result to make a later proposal valid.
- Route accepted dream `Add` proposals through that constructor before their
  candidate topology is persisted. Proposal acceptance, candidate isolation,
  promotion, reports, reversals, builtins, loading and historical parts retain
  their current behavior; loading never invokes the constructor or rewrites
  persisted instructions.

## Capabilities

### New Capabilities

<!-- None. -->

### Modified Capabilities

- `peer-cognition`: Dream-added peers retain the canonical equal-peer
  instruction framing through acceptance and persistence.

## Impact

The change is limited to the shared framework instruction construction in
`packages/kuru-core` and the runtime dream `Add` path and focused regressions
in `packages/kuru-runtime`. It changes no public provider, tool, memory schema,
candidate-promotion, builtin identity, model-authority, or history API, and
adds no dependencies. `ordered-dolt-migrations` may change the native memory
test backend exercised by acceptance, but it is not a semantic prerequisite
for this instruction correction.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [x] agent-behavior — prompts, tools, model routing, or agent output shape
