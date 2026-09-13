## ADDED Requirements

### Requirement: Actual published Windows installation acceptance

Each release SHALL exercise its actual published Windows package on a native
Windows runner in a fresh isolated user environment. The acceptance check MUST
start with empty Kuru data and engine caches, run an offline demo conversation,
resume the same session, inspect memory status and history, list sessions, and
verify the extracted embedded engine and licenses against the checked-out
authoritative asset manifest.

#### Scenario: Cold published package persists and reopens
- **WHEN** the ordinary mise-installed release runs with the demo provider and an empty offline engine cache
- **THEN** it extracts the bundled engine, completes and resumes one durable session, reports the resulting memory state and history, and lists both turns without downloading another engine

#### Scenario: Published native cleanup is uncertain
- **WHEN** any owned verifier or application process cannot be confirmed stopped before isolated state cleanup
- **THEN** the native acceptance check fails and does not report successful verification
