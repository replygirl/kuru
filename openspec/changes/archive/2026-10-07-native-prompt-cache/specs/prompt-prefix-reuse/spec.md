## MODIFIED Requirements

### Requirement: Shared stable prompt prefix

Kuru SHALL place genuinely shared, stable mode and reviewed project instructions before actor identity, phase, topology, transcript and other mutable context in the effective provider instructions. It SHALL mark the end of common instructions with explicit request metadata. The native subscription connector SHALL project exact shared/local segments as ordered plain developer text blocks and derive a cache routing key from common instructions, sorted actually offered tools, model and effort. It SHALL NOT include actor identity or private context in that key or send rejected API cache controls on the native route. API-key/custom routes and legacy callers without metadata SHALL preserve their compatible transport shape. It SHALL keep actor-private histories, private summaries, state and native continuation scoped to their actor and SHALL NOT copy them into the common prefix. Common tool definitions SHALL have deterministic order and serialization. The projection SHALL preserve instruction authority and meaning; matching layout or key alone SHALL NOT establish a cache hit.

#### Scenario: Two actors in one project
- **WHEN** two actors use the same mode, reviewed project instructions and tool inventory
- **THEN** their effective instructions share the same leading common bytes and their tool-definition projections match, while each actor's identity and private context remains in its own request after that shared material.

#### Scenario: Mutable public context
- **WHEN** the public transcript or topology changes between turns
- **THEN** the stable leading instructions remain identical and the changed content appears only after that prefix.

#### Scenario: Native shared-prefix projection
- **WHEN** the runtime sends common instructions followed by changing actor instructions on the subscription route
- **THEN** the connector separates those exact segments into plain developer text blocks and retains the same routing key for matching common instructions, offered tools, model and effort, with changing instructions and private input outside the shared segment.

#### Scenario: Native continuation
- **WHEN** a part continues its native tool request
- **THEN** the common developer prefix appears once and saved native output remains in that part's protocol continuation with valid sizing ranges.
