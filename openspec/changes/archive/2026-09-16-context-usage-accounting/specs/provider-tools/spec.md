## ADDED Requirements

### Requirement: Preflight the effective provider request

The connector SHALL expose one pre-dispatch hook after the selected native input and actor-local pending continuation are assembled, while preserving private pending bytes behind its actor lock. The hook SHALL receive bounded non-content source measurements and the selected model fit policy, and MAY return a typed `ContextTooLarge` refusal. It MUST check the final request at the same construction boundary before HTTP so a second request build or later tool loop cannot bypass fit. It SHALL preserve current ordered call results, opaque continuation and the existing independent transport-byte cap.

#### Scenario: Two-call continuation
- **WHEN** two tool results complete one pending native call batch containing opaque reasoning
- **THEN** the preflight measures the exact continued request, rejects an oversized mandatory chain before HTTP, and never emits opaque content to runtime, trace, journal or UI.

### Requirement: Ordered fallible usage observation

The provider stream SHALL offer each usage observation to an awaited, fallible sink in local sequence order, including reports preceding terminal error. The final terminal report SHALL remain distinguishable from earlier observations and preserve absent versus explicit zero components. If the sink rejects durability, provider collection MUST fail instead of acknowledging usage and continuing with unaccounted work.

#### Scenario: Usage before stream failure
- **WHEN** a native stream emits partial usage followed by a terminal failure
- **THEN** the usage observer receives the partial report before failure, while no partial assistant completion or tool call is authorized.
