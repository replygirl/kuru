## ADDED Requirements

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

`pre_turn` hooks SHALL allow, deny, or rewrite only the pending actor input, and `pre_tool` hooks SHALL allow, deny, or rewrite only the proposed tool name and arguments. Hooks for one event MUST run sequentially in effective configuration order, with each valid rewrite becoming the next hook's input. Before external dispatch, Kuru MUST apply the ordinary input shape, budget, schema, tool-root, permission, workspace-authority, and mode-policy checks that apply to the exact final value. A prior grant or evaluation MUST NOT authorize a changed operation. Original durable user/raw input MUST NOT be rewritten.

#### Scenario: Ordered tool rewrite narrows authority

- **WHEN** two pre-tool hooks rewrite one proposed path in declaration order and the final path is denied by the existing evaluator
- **THEN** neither the original nor rewritten tool operation executes and the denial reflects the exact final operation

#### Scenario: Pre-turn refusal preserves original record

- **WHEN** a pre-turn hook denies an admitted user input
- **THEN** Kuru makes no provider or tool request, preserves the original durable input without replacing it with hook output, and reports the hook refusal truthfully

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

Every hook process SHALL use Kuru's owned connector launch boundary with a retained reviewed workspace, finite inherited compatibility environment, bounded pipe draining, and child/tree reaping on success, failure, timeout, cancellation, and caller loss. Lifecycle-hook execution and response handling MUST NOT recursively trigger another lifecycle hook. Documentation and diagnostics MUST describe hook commands as process authority and MUST NOT claim an OS sandbox.

#### Scenario: Cancellation during a pre hook

- **WHEN** a turn is cancelled while a pre-turn command holds stdout open and has a descendant process
- **THEN** Kuru terminates and reaps the owned tree, drains bounded output, releases the workspace capability, and performs no provider dispatch

#### Scenario: Hook starts Kuru internally

- **WHEN** a hook command invokes a Kuru operation that would otherwise cross a configured lifecycle boundary
- **THEN** the originating hook chain does not recursively invoke its own or another lifecycle hook through hook protocol handling

#### Scenario: Dream has only real tool events

- **WHEN** a dream participant returns a `dream_suggest` provider call
- **THEN** Kuru may run pre/post tool hooks with its real actor, session, operation, invocation, and call identities and no turn ID, while no user-turn or speaker-selection hook fires
