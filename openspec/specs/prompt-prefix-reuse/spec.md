# prompt-prefix-reuse Specification

## Purpose
Define which prompt material Kuru can place in a stable, shared leading segment across actor requests while preserving actor-private boundaries, and require provider-reported usage before claiming cached tokens or savings.

## Requirements

### Requirement: Shared stable prompt prefix

Kuru SHALL place genuinely shared, stable mode and reviewed project instructions before actor identity, phase, topology, transcript and other mutable context in the effective provider instructions. It SHALL keep actor-private histories, private summaries, state and native continuation scoped to their actor and SHALL NOT copy them into the common prefix. Common tool definitions SHALL have deterministic order and serialization. The reordering SHALL preserve the instructions' authority and meaning.

#### Scenario: Two actors in one project
- **WHEN** two actors use the same mode, reviewed project instructions and tool inventory
- **THEN** their effective instructions share the same leading common bytes and their tool-definition projections match, while each actor's identity and private context remains in its own request after that shared material.

#### Scenario: Mutable public context
- **WHEN** the public transcript or topology changes between turns
- **THEN** the stable leading instructions remain identical and the changed content appears only after that prefix.

### Requirement: Evidence-based cache reporting

Kuru SHALL base any cache-hit or cached-token claim on usage actually returned for that invocation, retaining a missing report distinct from an explicit zero. It MAY show matching request-prefix measurements as potential reuse, but SHALL NOT call such a match a cache hit or infer support for the Codex subscription route from direct-API documentation. Cost estimates SHALL continue to disclose unapplied cache-write pricing terms rather than treating them as zero.

#### Scenario: Identical prefix without cached usage
- **WHEN** consecutive requests have a matching common prefix but the provider reports no cached-input component
- **THEN** Kuru may report the prefix match but does not report cached tokens or savings.

#### Scenario: Observed cached tokens
- **WHEN** a provider response reports cached-input tokens, including explicit zero
- **THEN** the invocation and session accounting retain that observed value with its route and do not add it to total input a second time.
