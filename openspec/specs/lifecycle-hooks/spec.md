# lifecycle-hooks Specification

## Purpose
Define bounded, ordered lifecycle hooks that observe or shape pending work under Kuru's existing trust and permission authority while preserving settled turns, tool effects, and equal-peer speaker selection.

## Requirements

### Requirement: Bounded typed lifecycle-hook protocol

Kuru SHALL support ordered configured `pre_turn`, `post_turn`, `pre_tool`, `post_tool`, and `speaker_selected` command hooks through one versioned, event-specific, single-request protocol. Each invocation MUST receive only bounded projected event data and stable opaque identifiers on stdin, and MUST return exactly one bounded response permitted for that event. Hook count, request bytes, stdout, stderr, duration, aggregate per-turn executions, and aggregate annotation bytes MUST have validated finite limits. Unknown fields or variants, invalid UTF-8, trailing values, output beyond the bound, and an event-incompatible result MUST fail closed without using partial output.

An operation's aggregate time limit SHALL count active hook command wall time, including cleanup and reap; overlapping commands SHALL consume their shared active interval once, while provider inference and ordinary tool work SHALL consume none. Aggregate count, time, and annotation-byte exhaustion MUST prevent new pre-event dispatch or record a separate post-event failure without changing settled work. Effective limits MUST join the reviewed hook authority manifest.

#### Scenario: Malformed pre-hook output

- **WHEN** a pre-tool hook returns a valid object followed by trailing data or an event-incompatible annotation
- **THEN** Kuru rejects the complete response before tool permission evaluation or dispatch and uses no partial rewrite

#### Scenario: Bounded post-hook diagnostic

- **WHEN** a post-turn hook exceeds its stdout limit while stderr contains terminal controls and secret-like text
- **THEN** the settled answer remains unchanged and Kuru records only a bounded escaped hook failure without exposing raw output

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

Original durable user and raw input MUST NOT be rewritten. A pre-turn rewrite replaces the original in the current turn's provider requests. Wherever the rewritten input is durably retained in an actor's private history, it MUST be accompanied by a hook-provenance record, so hook-authored text is never stored as indistinguishable user speech. The provenance record MUST remain private: Kuru MUST omit it from every provider projection, including the current request, later turns' retained history and context compaction. Post-hook annotations are separate records and keep their own context eligibility.

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
- **THEN** no provider request carries the original input or the `kuru-hook` provenance record, the current request carries the rewritten input, and the private history still holds the record immediately before the rewritten input

### Requirement: Post-event hooks preserve settled work

`post_tool` and `post_turn` hooks SHALL observe only bounded projections of immutable settled results. They MUST NOT rewrite, replace, suppress, relabel, retry, or roll back the result, answer, effect, usage, receipt, or durable record. A post hook MAY return a bounded annotation recorded separately with hook identity and outcome. A post-tool annotation MAY accompany the unchanged result to the receiving actor; a post-turn annotation MAY become eligible only for later actor context. Every annotation MUST pass the receiving actor's existing visibility and context-fit policy, and omission MUST be reported without implicit provider dispatch.

#### Scenario: Post-tool failure after mutation

- **WHEN** a mutating tool settles successfully and its post-tool hook times out
- **THEN** the mutation, result, receipt, usage, and settled observation remain successful while a separate bounded hook failure is recorded

#### Scenario: Annotation exceeds later context

- **WHEN** valid post-event annotations do not fit the receiving actor's context budget
- **THEN** Kuru deterministically omits the eligible annotations, reports the omission, and neither changes the settled work nor starts a new turn

### Requirement: Speaker hooks cannot supervise peer selection

Kuru SHALL invoke `speaker_selected` only after the active mode policy selects and the runtime validates an eligible speaker. The hook MAY observe the immutable selection or stop its dispatch. It MUST NOT substitute an actor, alter eligibility or reason, request reselection, grant tool authority, or replace the mode policy. A stop MUST produce no provider dispatch for that selection and MUST preserve equal-peer topology.

#### Scenario: Attempted speaker substitution

- **WHEN** a speaker hook returns an actor different from the validated mode-policy selection
- **THEN** Kuru rejects the hook result and dispatches neither the selected nor proposed replacement actor

### Requirement: Hook process ownership and non-recursion

Every hook process SHALL use Kuru's owned connector launch boundary, with:
- a retained reviewed workspace
- a finite inherited compatibility environment
- bounded pipe draining
- child and tree reaping on success, failure, timeout, cancellation, and caller loss

Owned hook cleanup MUST signal before reap. It MUST complete, or be reported as unconfirmed, before:
- the cancelled or completed turn or dream returns
- tool-host shutdown returns, and therefore before the project writer lease can be released

After the root process exits, reaping and draining stdout and stderr MUST share one post-exit cleanup bound and MUST stay within the remaining invocation deadline. A pipe still held after that bound MUST fail the hook closed without using partial output. Caller loss during the drain MUST stop it at the next poll, so the post-exit tail never exceeds the host's quiescence bound.

Lifecycle-hook execution and response handling MUST NOT recursively trigger another lifecycle hook. When the internal hook-origin marker suppresses configured hooks, Kuru MUST report each suppressed configured hook as a typed `suppressed` outcome at its lifecycle boundary instead of silently skipping it.

On Windows, hook launches MUST remove an inherited `PSModulePath` only when the command explicitly selects stock Windows PowerShell. They MUST NOT receive the ToolHost stock-shell module bootstrap.

Documentation and diagnostics MUST describe hook commands as process authority and MUST NOT claim an OS sandbox.

#### Scenario: Cancellation during a pre hook

- **WHEN** a turn is cancelled after a pre-turn command has started and spawned a descendant process
- **THEN** Kuru terminates and reaps the owned tree, drains bounded output, releases the workspace capability, and performs no provider dispatch before the cancelled operation returns

#### Scenario: Escaped descendant holds stdout

- **WHEN** a hook's root process exits after starting a descendant that leaves the owned process group and keeps stdout open
- **THEN** the hook fails closed within its deadline and the operation does not wait for the escaped descendant

#### Scenario: Hook starts Kuru internally

- **WHEN** a hook command invokes a Kuru operation that would otherwise cross a configured lifecycle boundary
- **THEN** the originating hook chain does not recursively invoke its own or another lifecycle hook through hook protocol handling, and the nested operation reports its configured hooks as suppressed

#### Scenario: Dream has only real tool events

- **WHEN** a dream participant returns a `dream_suggest` provider call
- **THEN** Kuru may run pre/post tool hooks with its real actor, session, operation, invocation, and call identities and no turn ID, while no user-turn or speaker-selection hook fires

#### Scenario: Cancellation after root exit

- **WHEN** a caller is cancelled after the hook root exits while an escaped descendant still holds its output open
- **THEN** the hook worker stops draining and settles within its shared post-exit bound, quiescence reports no unconfirmed cleanup, and Kuru does not wait for the escaped descendant
