# Spec Delta

## ADDED Requirements

### Requirement: Current Sol and Luna metadata

Kuru SHALL include sourced exact-ID records for `gpt-6.1-sol` and `gpt-6-luna` on the official Responses and native subscription routes. Official API limits and prices MUST NOT become unsupported subscription limits or billing claims. Provider advertisements SHALL retain precedence and custom routes SHALL inherit no official facts. Earlier exact-ID records SHALL remain available.

#### Scenario: New API models receive verified metadata

- **WHEN** the official Responses route discovers either new exact model ID
- **THEN** absent limits and prices are enriched from verified sources without changing advertised model identity, efforts, or facts

#### Scenario: Subscription metadata remains route specific

- **WHEN** the native subscription route discovers either new exact slug
- **THEN** API-equivalent prices are labelled and undocumented subscription limits and tokenizer mappings remain absent

### Requirement: Fresh discovery after login and on open

Kuru SHALL enumerate its fixed native ChatGPT subscription catalog after either successful browser or device login, without activating workspace-selected provider or memory authority or using a Responses API key. Listing SHALL NOT send inference requests. Failed listing SHALL preserve successful authentication and show safe retry guidance without provider response bodies. Ordinary startup and explicit model listing SHALL continue fetching the selected provider's current catalog, preserving newly advertised IDs and effort strings without requiring a Kuru update. Explicit model IDs SHALL remain exact selections; Kuru SHALL NOT introduce family shorthand aliases as part of this change.

#### Scenario: Successful login reveals available models

- **WHEN** browser or device authentication completes and native listing succeeds
- **THEN** login reports success and presents discovered model IDs without sending a completion

#### Scenario: Listing failure does not undo login

- **WHEN** authentication succeeds but post-login listing fails
- **THEN** Kuru retains authentication, reports successful login, and offers `kuru models` retry guidance without leaking a response body

#### Scenario: Newly advertised model appears on the next open

- **WHEN** the provider catalog changes between independent invocations
- **THEN** the next invocation fetches and exposes the new model and its advertised efforts without application or static catalog updates
