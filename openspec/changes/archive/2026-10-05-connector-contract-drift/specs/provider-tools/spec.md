## MODIFIED Requirements

### Requirement: Supported OpenAI transport

The application SHALL support native ChatGPT OAuth authentication and direct
OpenAI subscription inference, and Responses API authentication through a
configurable environment variable, with dynamically discovered models and
available effort settings. Login, refresh, status and logout MUST manage only
Kuru's checked private credentials. The application MUST NOT require or embed
the Codex CLI/app-server or switch billing routes after authentication failure.
An automatic-ancestor Responses route MUST receive matching workspace approval
before Kuru reads its named environment variable or contacts its API base;
fixed ChatGPT login and logout MUST NOT read an unrelated workspace-selected
API-key environment variable. The connector MUST report incompatibility only
for complete bounded responses that positively contradict a required contract
semantic; ordinary authentication, transport, incomplete-stream, optional-field,
and model-output validation failures MUST retain their existing behavior. A
positively incompatible completed tool response MUST stop before ambiguous tool
dispatch and MUST NOT cause provider redispatch or billing-route fallback.

#### Scenario: Future effort setting
- **WHEN** the provider advertises an unfamiliar effort value
- **THEN** the application preserves it and can send it without requiring a code change.

#### Scenario: Standalone authentication
- **WHEN** Kuru is installed without another agent harness
- **THEN** native browser/device login, credential status and direct authenticated requests work without that harness.

#### Scenario: Invalid callback or cancelled login
- **WHEN** a callback has an invalid state or a pending login is cancelled
- **THEN** Kuru preserves its previous credentials and releases the owned callback resources.

#### Scenario: Concurrent refresh and logout
- **WHEN** requests race token rotation or logout
- **THEN** validated rotation is reused without overwriting a newer login or resurrecting a logged-out session.

#### Scenario: Pending workspace API route
- **WHEN** an unapproved automatic ancestor selects a Responses credential route
- **THEN** login and logout still use their fixed ChatGPT route, while an active Responses status check requires approval; none reads the selected API-key environment value before its applicable preflight permits it.

#### Scenario: Completed incompatible tool response
- **WHEN** a complete native response positively contradicts required tool identity or terminal reconciliation semantics
- **THEN** the connector returns a fixed bounded reason before ToolHost and does not retry or switch authentication routes.
