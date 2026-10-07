# Proposal

## Why

The native subscription evaluation reported zero cached-input tokens despite a large byte-identical common instruction prefix across parts and rounds. Kuru combines common and mutable instructions in one changing top-level string and does not supply a shared cache routing key. Native controls show opportunistic reuse, and the native endpoint rejects the explicit breakpoint documented for the API route; API documentation cannot establish native compatibility.

## What Changes

Carry an explicit common-instruction boundary from runtime construction to the connector. On the subscription route, serialize plain shared and actor-local developer text blocks and derive a routing key from common instructions, sorted actually offered tools, model and effort. Preserve API-key and legacy request shapes, instruction authority and actor-private continuation. Require positive provider-reported cached input across actors in a bounded ordinary native harness evaluation before declaring the roadmap outcome complete.

## Capabilities

### New Capabilities

### Modified Capabilities

- `prompt-prefix-reuse`: define the explicit shared boundary and compatible native projection while retaining truthful observed cache reporting.

## Impact

Shared completion request metadata, runtime instruction construction, Responses request serialization and native input sizing, connector/runtime regression tests, bounded native acceptance, and provider documentation. No authentication route changes, new dependencies, persisted private histories, or release publication.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [x] agent-behavior — prompts, tools, model routing, or agent output shape
