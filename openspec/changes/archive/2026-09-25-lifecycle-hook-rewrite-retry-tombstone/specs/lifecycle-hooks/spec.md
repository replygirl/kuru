## MODIFIED Requirements

### Requirement: Pre-event rewrites receive final validation

`pre_turn` hooks SHALL allow, deny, or rewrite only the pending actor input.

`pre_tool` hooks SHALL allow, deny, or rewrite only the proposed tool arguments:
- A `pre_tool` rewrite MUST repeat the unchanged proposed tool name.
- A rewrite that changes the tool name MUST fail closed before any later hook, permission evaluation or dispatch.

Hooks for one event MUST run sequentially in effective configuration order, with each valid rewrite becoming the next hook's input.

Before dispatch, Kuru MUST admit a call the runtime dispatches itself only when its exact final name is among the tools offered to that actor for that exact request and phase. This covers every cognitive call, every deliberation call and every dream call, whether hook-rewritten or model-proposed. Every other call MUST pass the ToolHost admission and permission evaluation of its exact final name and arguments. Kuru MUST then apply every ordinary check that applies to the exact final value:
- input shape
- budget
- schema
- tool-root
- permission
- workspace-authority
- mode-policy

A prior grant or evaluation MUST NOT authorize a changed operation.

Original durable user and raw input MUST NOT be rewritten. The user-facing public transcript, session history and session export MUST keep the original input of a rewritten turn. A full memory export MAY carry the private provenance records and the rewritten input, but never the original.

A pre-turn rewrite MUST replace the original in every provider projection of that turn's input:
- the rewritten turn's own requests
- every later turn's projection of the public transcript, for every actor, including an actor that did not take part in the rewritten turn
- resumed sessions and forks that inherit the turn
- context compaction

The provider-facing view MUST be consistent with what the model received, and MUST NOT disclose that a hook rewrote the input. The projection MUST follow the turn's latest attempt: a retried attempt that is rewritten again replaces the retained rewrite, and a retried attempt that sends the original MUST make later projections use the original.

Wherever the rewritten input is durably retained in an actor's private history, it MUST be accompanied by a hook-provenance record, so hook-authored text is never stored as indistinguishable user speech. Kuru MUST also retain durable turn-scoped rewrite provenance that carries the rewritten input without the original. It MUST retain that provenance, or clear it for an attempt that sends the original, before provider dispatch.

Both provenance records MUST remain private: Kuru MUST omit them from every provider projection. Post-hook annotations are separate records and keep their own context eligibility.

#### Scenario: Ordered tool rewrite narrows authority

- **WHEN** two pre-tool hooks rewrite one proposed path in declaration order and the final path is denied by the existing evaluator
- **THEN** neither the original nor rewritten tool operation executes and the denial reflects the exact final operation

#### Scenario: Pre-turn refusal preserves original record

- **WHEN** a pre-turn hook denies an admitted user input
- **THEN** Kuru makes no provider or tool request, preserves the original durable input without replacing it with hook output, and reports the hook refusal truthfully

#### Scenario: Tool name substitution is refused

- **WHEN** a pre-tool hook for a deliberating peer's `remember` call returns a rewrite naming `a2a_send`
- **THEN** Kuru settles the original call as a failed hook outcome, runs no later pre-tool hook, requests no approval, and dispatches neither tool

#### Scenario: Call outside the offered phase tools

- **WHEN** a deliberating peer proposes a tool that deliberation does not offer
- **THEN** Kuru settles that call as refused without permission evaluation or dispatch, even when no pre-tool hook is configured

#### Scenario: Rewritten pre-turn input keeps provenance

- **WHEN** a pre-turn hook rewrites the input and the turn settles
- **THEN** each participating actor's private history retains a `kuru-hook` pre-turn provenance record immediately before the rewritten current input, and the public transcript retains the original input

#### Scenario: Provenance record is never sent to a provider

- **WHEN** a pre-turn hook rewrites the input, the turn settles, a later turn runs and the part's history is compacted
- **THEN** no provider request message carries the `kuru-hook` provenance record, the current request carries the rewritten input, and the private history still holds the record immediately before the rewritten input

#### Scenario: Later projections never carry the original input

- **WHEN** a pre-turn hook rewrites one targeted turn and afterwards a later turn targets a different actor, the part's history is compacted, the session is resumed and a fork of the rewritten turn runs a turn
- **THEN** no provider request message or instructions carry the original input, later public-transcript projections carry the rewritten input in its place, and the user-facing history still shows the original input

#### Scenario: Retry without a rewrite projects the original

- **WHEN** a pre-turn hook rewrites a turn, the attempt stops after the rewrite is retained but before provider dispatch, and the retried attempt's hooks allow the original input unchanged
- **THEN** the retry sends the original input, later public-transcript projections carry the original input, and no provider request carries the earlier rewritten text
