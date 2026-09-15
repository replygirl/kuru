## ADDED Requirements

### Requirement: Staged Windows release acceptance

Each release SHALL exercise its exact staged Windows ZIP on a native Windows runner before public GitHub release promotion. The acceptance check MUST route the real mise GitHub backend through an isolated loopback fixture serving simulated release metadata and those exact candidate bytes, start with empty Kuru data and engine caches, run an offline demo conversation, resume the same session, inspect memory status and history, list sessions, and verify the extracted embedded engine and licenses against the checked-out authoritative asset manifest. This staged check MUST NOT be claimed as an actual public download; the package-owned published verifier remains available as a separate post-publication diagnostic.

#### Scenario: Cold staged package persists and reopens
- **WHEN** the ordinary mise installation path consumes the exact staged release ZIP with the demo provider and an empty offline engine cache
- **THEN** it extracts the bundled engine, completes and resumes one durable session, reports the resulting memory state and history, and lists both turns without downloading another engine

#### Scenario: Staged native cleanup is uncertain
- **WHEN** any owned fixture or application process cannot be confirmed stopped before isolated state cleanup
- **THEN** native acceptance fails and public release promotion remains blocked

#### Scenario: Staged bytes differ from the candidate
- **WHEN** the ZIP served to mise or its checksum differs from the complete candidate artifact selected for publication
- **THEN** acceptance fails rather than substituting rebuilt or public bytes

## REMOVED Requirements

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
