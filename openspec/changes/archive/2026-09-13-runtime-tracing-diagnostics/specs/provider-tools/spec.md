## ADDED Requirements

### Requirement: Typed retry observation

Provider retry handling SHALL emit only literal operation category, attempt, safe status classification, delay or exhaustion outcome, and elapsed time to the application's operational tracing target. It MUST NOT emit request payloads, headers, endpoints, provider diagnostic text, or error chains.

#### Scenario: Retryable rejection
- **WHEN** a retryable provider rejection schedules a later attempt
- **THEN** the diagnostic observation identifies the typed retry outcome without admitting remote text.
