# mode-policy-dispatch Specification

## Purpose
Make mode roles, peering, flow and facing govern actual runtime decisions while preserving the four built-in modes' behavior. Validate policy outputs before dispatch or publication, retain runtime authority over permissions and budgets, and publish each selected profile only with its durable configuration, topology and session.

## Requirements

### Requirement: Policy-owned runtime dispatch preserves built-in behavior

The runtime SHALL consume the validated Roles, Peering, Flow, and Facing
components of the selected built-in profile for fresh topology seeds, required
role coverage, deliberation recipients, direct peer and relationship admission,
one-hop consultation, shared contributions, and speaker selection. It SHALL
validate every policy result against live topology, actual pending inputs,
nonself active edges, canonical relationships, and eligible speaker rules
before the corresponding event publication, mail enqueue, provider request, or
state change. When the caller supplies a valid explicit target, the initial
recipient set SHALL be exactly that target and Facing SHALL preserve its
precedence; a policy result that redirects it SHALL be rejected.
The runtime SHALL retain provider and tool execution, P6 permission evaluation,
P7 accounting, budgets, cancellation, exact retry, persistence-before-
publication, and equal-peer authority.

#### Scenario: Four built-in golden dispatches

- **WHEN** deterministic turns exercise target, focus, activation, continuity,
  cold ties, peer delivery, relationships, and one-hop consultation in each of
  the four built-in modes
- **THEN** normalized requests, recipients, contributions, speaker/reason,
  limits, event sequence, and durable outcome match the pre-extraction golden
  baseline, apart from approved earlier phase representations.

#### Scenario: Rejected peer policy edge

- **WHEN** a test-only Peering policy denies a formerly valid direct edge
- **THEN** the runtime produces the existing truthful refusal before mail,
  delivery event, recipient provider request, or tool side effect occurs.

#### Scenario: Flow suppresses consultation

- **WHEN** a test-only Flow policy returns no recipient for a speaking
  consultation
- **THEN** no consultation is enqueued or reported delivered, while the
  existing turn and tool budget behavior remains bounded.

#### Scenario: A policy redirects an explicit target

- **WHEN** a policy selects a different initial recipient or speaker for a
  valid explicit target
- **THEN** the runtime refuses that selection before sending the targeted
  input to an unrelated actor.

### Requirement: Mode dispatch policy seams are effective and bounded

The runtime SHALL permit an internal test-only validated profile injection for
an existing mode without adding a shipping mode, configuration, UI control, or
general policy executor. An alternate Roles policy SHALL affect fresh seed and
required-role validation; an alternate Flow policy SHALL affect initial
selection only among eligible active identities and later selection only from
actual pending input; and an alternate
Facing policy SHALL affect the eligible speaking actor, reason, and resulting
provider request. A policy MUST NOT invent a recipient outside those validated
sets, fan out consultation,
appoint a supervisor, grant tool authority, or bypass runtime validation.

#### Scenario: Alternate facing selection

- **WHEN** a test-only Facing policy selects a different live eligible draft
- **THEN** that actor and reason appear in the provider request and typed event,
  with no change to tool permission or execution authority.

#### Scenario: Invalid policy output

- **WHEN** a policy returns an inactive recipient, a nonpending later-round
  recipient, an invalid contribution, or an ineligible speaker
- **THEN** the runtime rejects the result before external dispatch or public
  publication and preserves its prior durable topology.

### Requirement: Dream role validation remains runtime-owned

Dream topology validation and undo SHALL use the selected profile's Roles
seeds, required roles, and accepted roles while preserving candidate-only
dream writes, reconciliation, retirement coverage, and reversible undo. P8
SHALL NOT route dream consolidation, visibility, context, namespace, or memory
policy decisions; those remain P9 work.

#### Scenario: Required role retirement

- **WHEN** a dream proposal or undo would leave the selected profile without a
  required role
- **THEN** runtime validation rejects it with no candidate promotion or live
  topology publication.

### Requirement: Speaker lifecycle observation preserves mode authority

After the selected mode policy returns and the runtime validates an eligible speaker, Kuru MAY run the configured `speaker_selected` hook chain with a bounded projection of that selection. Hooks MAY observe or stop dispatch only. They MUST NOT replace the speaker, alter eligibility or reason, request another selection, mutate topology, or grant provider or tool authority. Hook output MUST be validated before any speaker event publication or provider request.

#### Scenario: Speaker dispatch is stopped

- **WHEN** a speaker hook validly stops an otherwise eligible selection
- **THEN** Kuru reports the stopped hook outcome and performs no provider request without selecting a substitute speaker

#### Scenario: Mode-invalid hook result

- **WHEN** a hook attempts to replace the selected speaker or reason
- **THEN** runtime validation rejects the result before speaker publication or dispatch and preserves the mode policy and topology
