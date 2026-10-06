## ADDED Requirements

### Requirement: Separate chunk-safe partial shell presentation

An admitted shell call MAY expose a separately bounded partial preview consisting only of bytes already released by the existing stateful redaction scanner. Preview capture SHALL preserve scanner state across pipe reads, bound head/tail and aggregate retained updates, coalesce without blocking process output drainage, and visibly mark truncation or omitted updates. It SHALL NOT finalize incomplete scanner input, independently reproject raw read chunks or expose private operational diagnostics before their existing EOF boundary. Preview consumption SHALL NOT change tool output, exit classification, permission, process registration, cancellation, EOF or owned cleanup behavior.

#### Scenario: Recognized secret split across pipe reads
- **WHEN** a recognized credential spans multiple reads while a card is visible
- **THEN** no partial or settled frame exposes any unsafe secret fragment.

#### Scenario: Slow or absent preview consumer
- **WHEN** a consumer lags or disconnects while shell output continues
- **THEN** output drainage and owned cleanup remain independent, presentation storage stays bounded and omissions are visible.

#### Scenario: Incomplete timeout diagnostic
- **WHEN** timeout cleanup has not observed stderr EOF
- **THEN** the operational diagnostic retains its existing pending-EOF contract even when a separate safe partial card preview exists.
