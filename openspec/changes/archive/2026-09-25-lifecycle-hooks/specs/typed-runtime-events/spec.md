## ADDED Requirements

### Requirement: Hook outcomes use bounded semantic projection

The runtime SHALL project hook start and terminal outcomes through typed bounded data that identifies the event kind, configured hook identity, related turn or call identity, and outcome classification without raw command paths, arguments, stdin, stdout, stderr, rewritten payloads, settled result bodies, or annotation bodies. Valid annotations SHALL remain separately typed hook output subject to their receiving actor's visibility and context-fit policy. Post-turn hooks SHALL run only after the answer is settled and SHALL report their bounded outcomes separately to the initiating caller without changing the completed answer or journal. Completed replay MUST return the stored settled output and MUST NOT execute a hook again; it need not reconstruct later post-turn observations as part of that immutable output.

#### Scenario: Hook failure is safe to render

- **WHEN** a hook fails with hostile stdout, stderr, path text, and terminal controls
- **THEN** the live trace, CLI, and TUI receive only the bounded projected failure and no hostile material; any outcome recorded before settlement in the journal or completed output is bounded too

#### Scenario: Exact retry replays hook outcome

- **WHEN** a completed turn is retried by its exact identity after post-turn observation
- **THEN** Kuru returns its stored settled output without rerunning the hook, provider, or tool; the separate post-turn observation is not presented as a new durable turn outcome
