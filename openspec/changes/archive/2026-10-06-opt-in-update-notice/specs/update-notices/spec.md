# Spec Delta

## ADDED Requirements

### Requirement: Bounded personal availability check

An opted-in interactive Kuru SHALL check only the fixed HTTPS latest-release checksum manifest with fixed non-identifying user agent, five-second request bound, bounded redirects and 64KiB body. Its owner-only 4KiB cache MUST store only schema, time, running version, stable latest version and fixed outcome/failure labels. Success and failure SHALL be due after24h, incompatible running version or invalid/future cache evidence.

#### Scenario: Offline or hostile manifest
- **WHEN** the request fails or the manifest is oversized, ambiguous, malformed or does not name exactly one stable host archive
- **THEN** no notice is produced, no authority or installation effect occurs and a bounded failure cache may suppress repeated attempts

#### Scenario: Valid stable newer host archive
- **WHEN** the fixed manifest names exactly one valid stable host archive newer than the running version
- **THEN** at most one bounded advisory names that version and the correct literal Kuru or package-manager upgrade command

### Requirement: Restored-terminal advisory lifecycle

Only an opted-in interactive TUI with terminal stderr SHALL activate notices. Startup and exit MUST NOT wait for network checks. After terminal restoration, Kuru MAY print one completed advisory to stderr; unfinished work MUST be cancelled without delaying exit. Headless and fixed commands MUST NOT check, print or create notice cache. Notices MUST NOT download candidates or invoke providers or installers.

#### Scenario: Held request during quit
- **WHEN** the opted-in interactive session quits while its check is held
- **THEN** terminal restoration and ordinary exit complete without awaiting the response and no incomplete notice is printed

#### Scenario: Headless command with personal opt-in
- **WHEN** run, config, fixed account, inspection, update, serve or internal modes are invoked with update.notice enabled
- **THEN** no notice request/cache is created and stdout retains its existing shape
