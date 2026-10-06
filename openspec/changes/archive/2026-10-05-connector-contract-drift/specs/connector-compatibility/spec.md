## ADDED Requirements

### Requirement: Positive evidence governs provider incompatibility

Kuru SHALL classify a native ChatGPT authentication, catalog, or inference response as protocol-incompatible only when a complete bounded response contradicts a required established semantic or omits a required field needed for safe interpretation. The diagnostic MUST use a finite fixed code and bounded actionable text, MUST NOT include remote body text or credentials, and MUST stop ambiguous tool dispatch without redispatching the request. A timeout, connection/read failure, incomplete stream, ordinary HTTP rejection, expired credential, invalid model-generated arguments, absent optional usage or reasoning, unknown harmless field, unfamiliar model ID or effort value MUST NOT by itself be classified as incompatibility.

#### Scenario: Completed contradictory tool identity
- **WHEN** a completed tool item lacks required identity or its terminal identity contradicts announcements for that same item
- **THEN** the connector returns a fixed incompatibility code before ToolHost receives the call, without retrying the completed provider operation or switching authentication routes.

#### Scenario: Completed stream contradicts terminal output
- **WHEN** a bounded successful terminal response contradicts previously visible text or another required item-ID reconciliation rule
- **THEN** Kuru reports a fixed stream incompatibility and does not accept or dispatch ambiguous output.

#### Scenario: Incomplete or operational response
- **WHEN** a response times out, fails while reading, ends before its terminal event, or the server rejects authentication or rate limits a request
- **THEN** Kuru retains the existing transport, authentication, retry, and account-failure behavior rather than claiming protocol incompatibility.

#### Scenario: Compatible provider evolution
- **WHEN** a complete response preserves required semantics but adds unknown metadata, advertises unfamiliar model or effort values, omits optional usage or reasoning, or uses an accepted terminal ordering and same-item identity announcement
- **THEN** Kuru preserves the supported output and does not report incompatibility.

### Requirement: Explicit bounded ChatGPT subscription canary

Kuru SHALL provide an operator-invoked `kuru canary --model MODEL` for the fixed ChatGPT subscription route. It MUST use only Kuru's native ChatGPT credentials, one bounded Kuru-owned no-tool prompt, and a bounded end-to-end deadline; it MUST NOT activate workspace tools, open or mutate project memory, or fall back to the Responses API-key route. Its versioned sanitized JSON report MUST identify the observed stages and one of `verified`, `incompatible`, or `unverified`; exit codes MUST be 0, 3, and 2 respectively, while ordinary CLI misuse retains exit 1. A canary that was not run or could not conclude because of credentials, network, account, or provider-operational state MUST be reported as unverified when a report is requested. The canary MUST NOT be a prerequisite for normal login, inference, CI, or release.

#### Scenario: Successful limited observation
- **WHEN** an explicit canary receives a valid catalog and completed compatible no-tool response
- **THEN** it reports verified for observed stages and identifies unobserved usage, refresh, tool-call, and reasoning stages instead of claiming they were proved.

#### Scenario: Incompatible or inconclusive account observation
- **WHEN** an explicit canary sees a positively incompatible completed response, or lacks usable Kuru authentication/network access
- **THEN** it emits only fixed sanitized report fields, reports incompatible with exit 3 or unverified with exit 2, and does not read another harness's credentials or alter ordinary route availability.

### Requirement: Deterministic connector contract evidence

Normal CI SHALL exercise native authentication, refresh, catalog metadata, stream reconciliation, tool calls, reasoning, usage, and route separation with isolated fake OAuth, HTTP, and SSE services and fake credentials. The fixtures MUST preserve item-ID terminal authority, visible-text agreement, same-item tool identity announcements, complete argument validation before dispatch, unknown compatible fields, and distinct subscription/API-key routes. CI MUST NOT require a live account, paid inference, another harness's credential store, or a successful live canary.

#### Scenario: Accepted and incompatible response pair
- **WHEN** the fake subscription service returns one compatible completion and one positively incompatible completed tool response
- **THEN** the compatible result settles, the incompatible result is classified with a fixed code before any tool effect, and the provider is not redispatched or switched to the API-key route.
