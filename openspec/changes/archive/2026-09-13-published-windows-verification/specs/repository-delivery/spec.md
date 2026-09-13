## ADDED Requirements

### Requirement: Package-owned published Windows verifier

The delivery package SHALL provide one native Windows verifier and mise task for
an exact published Kuru version and expected commit. The verifier MUST use the
ordinary `github:replygirl/kuru@VERSION` backend in isolated mise and user roots,
with no inherited release token, provider credential, proxy, OAuth store, lock,
or endpoint replacement. It MUST independently verify the published tag and
commit, complete release asset inventory, checksum manifest, Windows archive,
selected installed executable, embedded Dolt executable, and bundled licenses.

#### Scenario: Ordinary published installation
- **WHEN** the verifier runs with an exact published version, expected commit, checked-out asset manifest, and native mise executable
- **THEN** it selects, installs, locates, and executes that version through mise without a custom endpoint, a compiler-built Kuru executable, or a separately installed Dolt

#### Scenario: Publication identity disagrees
- **WHEN** the tag, resolved commit, asset inventory, checksums, archive, installed executable, engine, or licenses disagree with the exact expected release
- **THEN** verification fails rather than accepting mise installation alone as provenance evidence

### Requirement: Bounded published verification receipt

Published Windows verification SHALL bound command execution and output, await
owned cleanup, and emit a bounded JSON evidence receipt containing fixed safe
metadata only after isolated state has been removed. It MUST parse Kuru machine
results from stdout without requiring stderr to be empty.

#### Scenario: Verification completes
- **WHEN** every release, installation, runtime, persistence, and cleanup check succeeds
- **THEN** the receipt records exact version and commit identity, digests, command milestones, durable session observations, bundled-runtime observations, and confirmed cleanup without raw child output or credentials

#### Scenario: Child emits informational stderr
- **WHEN** an otherwise successful Kuru command emits an informational first-run notice on stderr while retaining its JSON stdout contract
- **THEN** verification accepts the machine result and keeps only bounded failure diagnostics if a later check fails
