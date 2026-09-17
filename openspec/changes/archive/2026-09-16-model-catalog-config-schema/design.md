## Context

Live connector discovery currently returns only identity and effort fields, and
the strict TOML configuration parser has no published schema. The P2 design
review and checked source snapshot establish that metadata facts have distinct
route, identity, and billing meanings.

## Goals / Non-Goals

**Goals:**

- Keep `ModelInfo` as the discovery boundary while adding serde-defaulted
  metadata that old JSON consumers can decode.
- Make every embedded fact independently attributable and safely absent.
- Give future context and cost work enough information to present estimates
  honestly without implementing those features now.

**Non-Goals:**

- This does not query a catalog over the network, calculate request cost,
  reserve context, track usage, or validate model account access.
- This does not introduce configuration permissions, a second parser, or an
  unbounded raw provider-response cache.

## Decisions

### Model records are keyed by route and model ID

The catalog uses an explicit route enum plus an exact model ID. A model ID alone
is rejected as a catalog key because official Responses, subscription Codex, and
custom bases have materially different limits and billing semantics. A global
model-ID map was rejected because it would let a custom base inherit official
claims it has not established.

### Provenance is generic and fact-level

`Sourced<T>` wraps verified data with a provenance kind and source details.
Metadata fields remain optional independently, rather than using a record-wide
source tag, because an advertised window can coexist with a pinned price and
absent capability. A single opaque metadata blob was rejected because callers
could not distinguish an assumption from a guarantee.

### Prices preserve future estimator conditions

Price components use decimal strings and retain Standard API tier conditions,
promotional expiry/provenance, and cache-write qualification. Floating point and
a three-number-only price record were rejected because both lose the information
needed to avoid a false flat estimate for long contexts or cache writes.

### Context fallback is resolver behavior, not a catalog record

`assumed_context_window_tokens` participates only after verified route facts and
the snapshot. This gives unknown models a usable labelled value without allowing
configuration to erase a known ceiling. Treating the setting as an override was
rejected because the confirmed scope calls it an assumption rather than a policy
that can replace verified provider information.

### JSON Schema is a public compatibility asset

The schema lives under VitePress public assets with a versioned filename and is
tested from Rust after TOML-to-JSON conversion. A generated-at-runtime or
JavaScript-owned schema was rejected because it could drift from the Rust parser
and would add tooling outside the package owners.

## Integration contract

`kuru-core` owns the serializable `ModelInfo` metadata types, catalog parser,
route key, and resolved-window API. `kuru-connectors` passes its route identity
and only validated advertised fields into that API after live discovery; it does
not expose raw provider payloads. The catalog is an embedded core asset with
fixtures that construct live records in memory. The docs app serves the static
schema asset unchanged from `public/`; Rust tests load that same file and
validate TOML-converted JSON against it. Custom API bases identify a distinct
route key, preventing SDK/provider IDs from reconciling with official facts by
string equality alone.

## Risks / Trade-offs

- [Pinned facts can become stale] → Include source URLs, checked dates, exact
  basis labels, and no availability inference; refresh in a separately reviewed
  catalog update.
- [JSON Schema cannot express all semantic validation] → Test parser/schema
  parity for structural cases and retain explicit native cross-field tests.
- [Public `ModelInfo` expansion requires many initializers] → Make metadata
  defaultable and update constructors mechanically with focused compiler/tests.
