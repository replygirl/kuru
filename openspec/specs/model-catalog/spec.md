# model-catalog Specification

## Purpose
Provide sourced, route-aware offline model metadata that enriches live discovery without asserting account availability or gating inference.

## Requirements

### Requirement: Route-aware sourced metadata

Kuru SHALL provide optional model context-window, maximum-output, price, and
capability facts with per-fact provenance. The embedded catalog SHALL be
bounded, versioned, validated at load time, and keyed by provider route plus
model ID; its records SHALL include a source URL and checked date for every
non-assumption fact. Prices SHALL use exact decimal USD-per-million-token
representations and SHALL remain absent when not verified.

#### Scenario: Official Responses record enriches a live model
- **WHEN** the official Responses route discovers a model whose ID has a valid
  matching snapshot record
- **THEN** missing live metadata is filled from the record with pinned-source
  provenance while the live ID, name, efforts, and default effort are retained

#### Scenario: A custom Responses base uses no official facts
- **WHEN** a Responses configuration selects a custom API base
- **THEN** a matching model ID SHALL NOT inherit official OpenAI snapshot limits,
  capabilities, or prices solely from that ID

#### Scenario: Invalid catalog data is rejected
- **WHEN** a catalog asset has a duplicate route-and-ID record, invalid decimal,
  missing source, nonpositive limit, or maximum output larger than its context
  window
- **THEN** catalog loading SHALL fail with a diagnostic instead of enriching a
  model with untrusted metadata

### Requirement: Live discovery and unknown models remain usable

Kuru SHALL treat live provider discovery as the source of account availability
and SHALL enrich it fact-by-fact in this order: validated route advertisement,
route-matched snapshot, then absence. It SHALL preserve all provider-supplied
model IDs and effort strings, including unfamiliar values, and SHALL NOT use a
snapshot as a live availability list or inference gate.

#### Scenario: Advertised facts take precedence
- **WHEN** a live discovery record provides a valid context window that differs
  from a route-matched snapshot
- **THEN** the enriched metadata SHALL expose the advertised value and
  advertisement provenance

#### Scenario: Unknown model is selected
- **WHEN** a provider advertises or an explicit configuration selects an unknown
  model ID with no verified price or context window
- **THEN** selection remains permitted, price remains absent, and no availability
  or inference failure is introduced by catalog lookup

### Requirement: Honest context-window fallback

Kuru SHALL resolve a missing context window to a labelled built-in conservative
assumption, or to an explicit configured assumption when one is present. The
configured assumption SHALL apply only after route advertisement and a
route-matched snapshot fail to provide a window; it SHALL NOT override a known
provider ceiling. Resolved assumptions SHALL be distinguishable from verified
facts by provenance.

#### Scenario: Configured fallback replaces the built-in assumption
- **WHEN** a selectable model has no advertised or snapshot context window and
  configuration supplies `assumed_context_window_tokens`
- **THEN** the resolved window SHALL use that positive configured value and be
  labelled as a configured fallback assumption

#### Scenario: Known window ignores configured fallback
- **WHEN** a model has a verified advertised or snapshot context window and
  configuration supplies `assumed_context_window_tokens`
- **THEN** the resolved window SHALL retain the verified value and provenance

### Requirement: Subscription API-equivalent price basis

Kuru MAY associate a Codex subscription model with an API-equivalent price basis
only when the live subscription slug exactly matches a documented mapping in the
pinned catalog. It SHALL label that basis as API-equivalent, SHALL keep
subscription context limits route-specific, and SHALL retain documented rate
tiers and cache-write limitations so a later estimator cannot silently flatten
them into a false flat price.

#### Scenario: Exact documented subscription slug receives a labelled basis
- **WHEN** the Codex route advertises an exact catalogued slug with a verified
  API-equivalent mapping
- **THEN** its price metadata SHALL identify the API basis and SHALL NOT claim a
  subscription invoice, quota, or availability guarantee

#### Scenario: Undocumented subscription slug remains unpriced
- **WHEN** the Codex route advertises a slug with no exact documented mapping
- **THEN** the model SHALL retain its live identity and have no snapshot price
  basis
