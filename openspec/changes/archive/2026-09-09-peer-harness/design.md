## Context

The project starts empty and must deliver a working terminal harness. Shared Rust interfaces and ownership are documented in `docs/implementation-contract.md`. The psychological frameworks are software organization metaphors; actor reports of internal state are modeled outputs, not evidence of sentience or clinical claims.

## Goals / Non-Goals

**Goals:** Equal persistent actors, unrestricted peer-to-peer addressing within a bounded execution budget, private part and relationship memories, inspectable framework dynamics, conventional chat/tool affordances, supported OpenAI authentication, extensible protocol/provider boundaries, and verifiable delivery.

**Non-Goals:** Clinical treatment; proving consciousness; federation trust or autonomous remote discovery; background daemons running indefinitely; publishing releases without a repository remote; cross-project collective memory in v1; arbitrary general-purpose worker trees.

## Decisions

1. Use a Rust workspace with separate core, connector, runtime and application crates. This separates provider/protocol churn from cognition and replaces the rejected plugin approach whose host owns the supervisory loop.
2. Use Tokio actor mailboxes and a neutral deterministic scheduler. Each actor may address any peer; no actor is a manager of the pool. Resource budgets remain centralized mechanical invariants, rather than an LLM supervisor making all decisions.
3. Store SQLite namespaces for project/session, part and canonical relationship identity. Retrieve only the active actor's accessible memories. A global concatenated memory was rejected because it defeats isolation and makes relationship identity cosmetic.
4. Treat role topologies as versionable framework data. Relationship membership is a validated set of 2–4 active parts, and relationship memories persist beyond temporary speaking activation. Jungian collective memory is project-scoped in v1 to keep consent and persistence boundaries explicit.
5. Dream in bounded rounds using private proposals. Validate additions and retirements, preserve each role and archive rather than delete retired actors. Keep previous state so topology changes can be reversed; unconstrained mutation and destructive culling were rejected.
6. Use Codex app-server as an authenticated inference transport, with supported login/status/logout subprocesses and built-in tools disabled. Keep direct Responses API and demo providers behind the same trait. Unsupported OAuth token copying was rejected; model capabilities are discovered at runtime to avoid stale hardcoded model lists.
7. Expose built-in tools through a host with canonical root checks, symlink checks and explicit write/shell opt-ins. MCP follows standard initialization and tool calls. A2A JSON-RPC crosses transport boundaries, while internal typed envelopes preserve the same peer message intent without loopback HTTP overhead.
8. Install from source immediately. Release installation/update consume explicit versioned release roots and checksum manifests; CI builds target archives and publishes only on version tags. Do not invent a public owner or download endpoint.

## Interactive acceptance

Exercise noninteractive demo runs and actual terminal input, model/effort/mode controls, cancel and session restoration. Use a deterministic renderer test to inspect narrow and normal terminal layouts. Clearly identify demo behavior and provider failures.

## Operational surface

Single local foreground process; SQLite under a user data directory. MCP subprocesses are children with bounded output/time. Optional A2A ingress binds loopback by default. CI tests on Linux and macOS; tagged releases carry platform archives and SHA-256 checksums. No external services are provisioned by this change.

## Integration contract

Provider and protocol tests use deterministic local peers for malformed messages, timeouts, model discovery, effort propagation, tool routing and disconnects. A real Codex transport smoke test is separate evidence and requires available authentication. Documentation states the distinction between protocol test coverage and live service acceptance.

## Agent behavior acceptance

Record actor event traces proving that peers route to one another without an LLM supervisor, that unrelated private memory never enters another actor prompt, that relationship history is scoped, and that dreaming preserves roles and is reversible. Model calls, tool calls and rounds must terminate at configured budgets even when peers keep requesting work.

## Risks / Trade-offs

- [Model cost and latency scale with active peers] → configurable concurrency, round and tool budgets; status visibility and cancellation.
- [Psychological terminology can be mistaken for clinical validity] → document the computational scope and avoid diagnosis or treatment claims.
- [Provider protocol changes] → dynamic discovery, narrow adapters and local protocol contract tests.
- [Shell commands escape cwd] → explicit opt-in documented as process authority, never represented as a sandbox.
- [Coverage alone can reward tautological tests] → acceptance ledger includes real CLI, SQLite, protocol peers and installer failure-path checks in addition to the numeric line threshold.
