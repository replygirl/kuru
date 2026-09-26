## MODIFIED Requirements

### Requirement: Explicit verified build inputs

Memory SHALL own a native mise preparation task for immutable pinned build inputs.
Cargo MUST select the archive for TARGET, validate local bytes and fail missing or
invalid inputs without downloading or producing an unbundled executable. Every
manifest asset MUST declare an explicit provenance, either `upstream` or `built`.
Preparing one target MUST NOT fetch, build or require the input of any other target,
and an upstream asset MUST NOT be substituted for a missing built asset or vice versa.

#### Scenario: Offline prepared build
- **WHEN** a valid target archive is imported or reused from an offline mirror
- **THEN** the build embeds exactly those pinned bytes including their licenses

#### Scenario: Invalid or mismatched input
- **WHEN** the selected target archive is absent, truncated, corrupt or unsafe
- **THEN** preparation or compilation fails explicitly without a host fallback

#### Scenario: Upstream targets ignore built entries
- **WHEN** an upstream target is prepared from a manifest that also contains an
  unpinned or unprepared built asset
- **THEN** preparation succeeds exactly as before without fetching, building or
  reading the built asset's inputs

#### Scenario: Missing provenance
- **WHEN** a manifest asset omits its provenance or declares an unknown one
- **THEN** both manifest parsers reject the manifest

## ADDED Requirements

### Requirement: Pinned source-built engine inputs

For a target without an upstream archive, memory SHALL describe a `built` asset whose
Dolt module version and checksum, third-party source archives, toolchain versions and
build recipe are pinned in the manifest. A package-owned mise task MUST build that
archive only on the declared build host from those pinned inputs, verify every
downloaded source by size and digest, and produce the exact archive layout the
manifest declares. Runtime provisioning MUST NOT build or download an engine.

#### Scenario: Build on the declared host
- **WHEN** the build task runs on linux-x64 for a pinned built asset
- **THEN** it produces an archive whose size and SHA-256, executable digest, license
  digest and notice digests equal the manifest pins

#### Scenario: Refused build host
- **WHEN** the build task runs on any host other than the declared build host
- **THEN** it fails before fetching or compiling anything

#### Scenario: Tampered source input
- **WHEN** a fetched Dolt module or source archive does not match its pinned checksum
- **THEN** the build fails without producing or publishing an archive

#### Scenario: Unpinned built asset
- **WHEN** preparation or compilation selects a built asset whose archive digest is not
  yet pinned
- **THEN** it fails with an instruction to run the build task on the build host and
  commit the reported pins

### Requirement: Third-party notices for built engines

A `built` asset MUST declare, and its archive MUST contain beside Dolt's `LICENSES`,
the license notices of statically linked third-party components taken from the pinned
inputs, each pinned by size and SHA-256. Upstream assets MUST NOT declare notices.
Runtime extraction MUST accept exactly the declared members and verify each notice's
size, and extraction of upstream archives MUST remain unchanged.

#### Scenario: Missing notice
- **WHEN** a built asset declares no notices or its archive lacks a declared notice
- **THEN** manifest validation or archive verification fails

#### Scenario: Notice on an upstream asset
- **WHEN** an upstream asset declares notices
- **THEN** both manifest parsers reject the manifest

### Requirement: Reproducible built-engine pins

A built asset's pins MUST come from the declared build host in CI, where two
independent builds from fresh directories produce byte-identical archives before the
pins are trusted. Once pinned, every CI build MUST verify its archive against the
committed pins and fail on any difference.

#### Scenario: Nondeterministic build
- **WHEN** the two independent builds produce different archive bytes
- **THEN** the workflow fails and reports both digests

#### Scenario: Drift from committed pin
- **WHEN** a CI build of a pinned built asset produces bytes that differ from the pin
- **THEN** the workflow fails and reports the observed and pinned digests
