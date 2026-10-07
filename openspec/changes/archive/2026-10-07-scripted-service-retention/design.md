# Design

## Context

The serving loop owns a JoinSet of attachments and already checks its retained
owner lock between bounded accepts. Once its starter was authenticated, an empty
JoinSet currently ends the loop immediately. Maintenance can independently
request retirement when its authenticated attachment is the only one remaining.

## Goals / Non-Goals

**Goals:** retain the exact checked service and Dolt engine across completed CLI
processes, with explicit bounded idle cleanup and prompt maintenance.

**Non-Goals:** provider supervision, session-driving control APIs, process-ID
recovery authority, persistent service installation or additional storage rows.

## Decisions

- Add a public bounded configuration value instead of an ambient environment
  lifetime override. Default 30 seconds makes sequential calls useful; explicit
  zero keeps existing fixture teardown immediate without a test-build policy.
- Start one absolute monotonic idle deadline only after all attachment tasks are
  joined. A live or incomplete exchange cannot time out the owner; new attachment
  acceptance resets the empty interval. Acceptance means an existing retained
  private transport task, including its bounded hello handshake. A rejected
  hello still joins before a fresh empty interval begins; unauthenticated traffic
  never marks the starter reached. Continuously empty lock rechecks do not reset
  the deadline, and the existing connection/frame bounds still apply.
- Keep lock rechecks and maintenance requests independent of the idle deadline.
  The final shutdown path stays unchanged. Sleeping once outside the accept loop
  was rejected because it could block attachment and maintenance.
- Append a bounded optional owner-launch argument, preserving previous nine,
  ten and eleven argument forms. Existing expected-instance validation remains
  strict; a dash only represents the explicitly absent optional value.

## Risks / Trade-offs

- [Risk] Fixtures wait for the product retention interval → common CLI fixture
  configuration explicitly selects zero; standalone acceptance uses the real
  thirty-second default and existing maintenance cleanup on failure. The
  deliberately cold delivery measurement also explicitly selects zero.
- [Risk] Timer reset hides perpetual unused ownership → deadline persists across
  lock rechecks and only restarts after an actual attachment lifecycle.
- [Risk] Retained engine consumes resources → maximum configured idle interval is
  300 seconds, default is 30, explicit maintenance still retires an unused owner.

## Operational surface

The existing private Unix socket/Windows pipe and independent native owner run
locally, with unchanged connection/frame bounds and bundled Dolt version/arches.
No listener address, network service, container or secret is added. Sequential
CLI callers retain the same authenticated project/instance/generation checks;
retention owns storage only, while each caller owns inference and tools.

## Integration contract

Core MemoryConfig and the published JSON schema share the integer 0..300 bound
and default30. The existing CLI snapshot forwards its selected value as an
optional internal owner argument; old argument forms remain accepted. No RPC
wire type, schema migration or provider request changes. Synthetic offline CLI
fixtures use the existing native process and checked managed attachments.
