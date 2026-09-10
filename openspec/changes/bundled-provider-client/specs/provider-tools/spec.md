## MODIFIED Requirements

### Requirement: Supported OpenAI transport

The application SHALL support Codex-owned authentication through supported
commands and app-server transport, and Responses API authentication through a
configurable environment variable, with dynamically discovered models and
available effort settings. Codex login, device login, logout, status, model
discovery and inference SHALL resolve the same verified bundled client by
default. A deliberately configured external client SHALL remain an explicit
override; an unavailable override MUST fail without silently substituting another
client. Kuru MUST NOT read, copy, migrate or print Codex credential stores, and
its inference configuration MUST retain provider-native tools disabled.

#### Scenario: Future effort setting
- **WHEN** the provider advertises an unfamiliar effort value
- **THEN** the application preserves it and can send it without requiring a code change.

#### Scenario: Client-owned authentication
- **WHEN** a user invokes a supported Kuru authentication command without an external-client override
- **THEN** Kuru delegates to the bundled official client using its supported authentication lifecycle and does not inspect or relocate tokens.

#### Scenario: Explicit external client
- **WHEN** a user deliberately configures an external client command
- **THEN** authentication and inference use that client consistently, with an actionable failure if it cannot be started or is incompatible.

#### Scenario: Restricted inference after bundling
- **WHEN** the bundled client receives a peer inference request
- **THEN** Kuru retains the peer's isolated context, dynamic model/effort values and structured tool proposals while Codex-native execution capabilities remain disabled.
