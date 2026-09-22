## MODIFIED Requirements

### Requirement: Effective-request context fit

Before each inference dispatch, Kuru SHALL estimate the selected effective connector request after native pending continuation and current receipts are assembled. For a route/model with a sourced supported tokenizer and fully represented text, Kuru SHALL derive a labelled local tokenizer estimate from the final serialized request with a documented allowance for provider structure. When that route has saved native output, Kuru SHALL estimate every retained native output-item range across successive pending hops with the labelled byte heuristic while tokenizing the remaining final serialized body, without changing the transmitted request. Unknown/custom routes and unsupported mappings SHALL retain a visibly labelled historical byte heuristic plus the structural allowance. Kuru MUST NOT describe any estimate as an exact provider token count or a guaranteed upper bound. Instructions, tool schemas, current user input, required receipt/call chain and native pending bytes SHALL be mandatory. It MAY omit only older optional history rows as whole units, without mutating durable history, and SHALL report actual omitted row counts and source labels. If mandatory material plus output reserve exceeds the selected model window, it MUST refuse before network dispatch. Unknown model windows SHALL use a documented, visibly labelled conservative assumption rather than block inference solely for missing metadata. Byte transport limits remain independently enforced. Normal inference SHALL NOT call a remote token-count endpoint to decide fit.

#### Scenario: Oversized native continuation
- **WHEN** encrypted pending continuation and matching tool results exceed a small configured window even though visible messages alone appear to fit
- **THEN** the connector refuses before HTTP, preserves the actor-private continuation, and neither tool results nor opaque bytes are silently dropped.

#### Scenario: Optional history is omitted
- **WHEN** old visible history must be reduced to fit a request
- **THEN** complete older rows are omitted only from that request, current receipts remain intact, persisted rows remain unchanged, and the reported omission count equals the rows actually omitted.

#### Scenario: Sourced tokenizer request
- **WHEN** the final selected text request uses a supported route and catalog model with a verified encoding mapping
- **THEN** the preflight uses the pinned tokenizer-derived estimate and structural allowance on that final request, labels its provenance and checks it with the output reserve before dispatch.

#### Scenario: Unsupported or opaque request
- **WHEN** the selected request uses an unmapped model, custom Responses base or opaque pending output items
- **THEN** the preflight labels its whole-body byte fallback or mapped-route mixed native estimate respectively, without deleting transmitted pending bytes, changing model choice or adding a remote counting call.
