# embedded-runtime Specification

## Purpose
Make every Kuru executable self-contained for persistent offline memory by
embedding its verified target-specific full-Dolt archive and licenses, with
explicit build preparation and local-only runtime provisioning.

## Requirements

### Requirement: Complete executable runtime

Every ordinary Kuru executable SHALL contain its target-specific pinned full-Dolt
archive and upstream licenses. A fresh installed executable MUST initialize and
persist memory without a separately installed Dolt or runtime engine download.

#### Scenario: Offline first conversation
- **WHEN** only Kuru is installed with empty memory and engine cache and no network
- **THEN** a demo conversation persists and remains visible after reopening
- **AND** the matching verified engine and licenses are extracted locally

#### Scenario: Installation and update preserve completeness
- **WHEN** Kuru is installed through direct, mise or source installation or updated
- **THEN** the resulting executable carries the engine required for an offline
  first launch with an empty extraction cache

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

### Requirement: Safe local extraction

Runtime provisioning MUST retain bounded archive and payload validation, private staging, cancellation cleanup, an exact-version check of every freshly extracted engine before its activation, and atomic activation. Existing cache verification MUST retain checked directory and payload handles while reading every byte for the pinned digests, and MUST revalidate their names and identities before returning the verified pathname; it launches no process. This is a deliberate trade: an existing cached executable whose bytes match the pinned digest yet cannot run, or reports another version, is no longer detected by that verification and surfaces instead when the database server fails to start; the install-time probe and the Dolt-version-keyed cache directory are the mitigations. An explicitly configured `dolt_binary` is not a managed cache and is outside this extraction requirement; it remains version-probed on every open. Independent warm-cache verification SHALL proceed without the exclusive installation lock. An absent cache MUST be rechecked after acquiring that lock, and extraction and publication MUST retain it through their existing checked completion or recovery boundary, including the exact-version probe of the freshly extracted engine before it is activated. Runtime provisioning SHALL have no runtime HTTP engine-download path.

#### Scenario: Corrupt cache or archive
- **WHEN** an embedded archive or existing cache fails integrity validation
- **THEN** Kuru executes no unverified payload and preserves existing installed data

#### Scenario: Concurrent warm cache verification
- **WHEN** independent callers open the same existing valid cache while the installation lock is held or another warm verification is running
- **THEN** each caller verifies the complete pinned payload without waiting for the installation lock, and none re-probes its already-installed version

#### Scenario: Concurrent or cancelled first use
- **WHEN** extraction callers race or a caller is cancelled
- **THEN** staging remains owned until work stops and another caller observes only a fully verified, version-probed, activated runtime

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

### Requirement: Install stage teardown ordering

Once runtime provisioning has created a private install stage under the exclusive installation lock, it MUST release that lock only after the stage is resolved, on every exit that follows: publication, an error before publication, caller cancellation, unwinding, and the end of activation recovery. The stage is resolved in one of three ways. It is removed through the checked stage removal. It is retained after a refused or uncertain removal, and that retention is reported to diagnostics and recorded in a `.leftovers` receipt that names the stage and whether its engine was published. Or, for a failed activation, it is deliberately preserved as evidence and named in the returned error. A removal failure of such a stage MUST NOT be discarded silently, and a stage whose removal is uncertain MUST NOT be deleted again. Teardown on a cancelled or unwinding owner MAY block its thread only for the existing bounded stage-cleanup window. A stage counts as created once its owner-only private directory exists; a container abandoned because that directory could not be created is outside this requirement.

The receipt MUST record the first cause, its native OS error when one exists, and the attempt count and elapsed time of the bounded recovery window when that window ran. It MUST also record the descendant of the stage whose removal refused, relative to the stage, when the checked removal names one. On Windows, when the stage's engine was probed, the receipt MUST record the probe child's process id and creation time, and whether a process object with that exact identity could still be opened when the refusal was recorded (`retained`, `released`, or `unknown` with its cause). Recording these facts MUST NOT change how the stage is resolved, and a failure to observe one of them MUST be recorded rather than fail the release.

#### Scenario: Held stage on a cancelled or failed installation
- **WHEN** an installation is cancelled or fails before publication while a handle inside its private stage refuses removal
- **THEN** the stage stays on disk, its retention is reported and receipted with `published: false` while the installation lock is still held, the receipt and diagnostic name the retained stage, an error returned through the lease names it, and the lock is released afterwards

#### Scenario: Retained stage after publication
- **WHEN** a verified engine is published but its private stage cannot be removed
- **THEN** the retained-stage receipt and diagnostic are written before the installation lock is released, and the open still succeeds

#### Scenario: Unheld stage
- **WHEN** an installation publishes, fails or is cancelled with nothing holding its stage
- **THEN** the stage is gone before the installation lock is released

#### Scenario: Refusing descendant and probe child are recorded
- **WHEN** a probed stage's removal is refused by one of its descendants
- **THEN** the receipt names that descendant relative to the stage and, on Windows, the probe child's identity and whether its process object was still open at the refusal, and the stage is resolved exactly as it would be without them

### Requirement: Leftover stage collection

Every acquisition of the installation lock by runtime provisioning MUST attempt each `.leftovers` receipt present at that moment once, before any other work under the lock, through one checked removal of the stage it names. A warm open MUST NOT wait for the lock. It attempts the receipts only when the lock is free, and a busy lock leaves every receipt and stage unchanged. A refused or uncertain removal MUST leave both the stage and its receipt, MUST NOT be retried within the same sweep, and MUST be recorded in that receipt under the lock: it increments `sweep_refusals` and replaces `last_sweep_refusal` with the refusal's phase, cause, native OS error, refusing descendant, and (on Windows) the probe child's state at that refusal. The receipt's other fields MUST be kept, and its `version` stays `1`. When a sweep leaves no receipt and the receipts folder holds nothing but an empty `staging` directory, the sweep MUST attempt one checked removal of the receipts folder under the lock. A refusal leaves the folder for the next sweep and is reported to diagnostics. A `staging` directory that still holds a record MUST NOT be removed. A warm open whose lock attempt fails with an error, as opposed to a busy lock, MUST report that to diagnostics and MUST NOT write anything.

#### Scenario: Next acquirer attempts each receipt
- **WHEN** a receipted stage exists and runtime provisioning takes the installation lock on a cold or a warm open
- **THEN** it attempts that stage's removal once before any other work under the lock, and on success removes the stage and then its receipt

#### Scenario: Busy lock on a warm open
- **WHEN** a warm open finds a receipt while another holder has the installation lock
- **THEN** the open does not wait, the stage and receipt are unchanged and record no refusal, and the next acquisition attempts the receipt

#### Scenario: Refused sweep
- **WHEN** a sweep's checked removal of a receipted stage is refused or uncertain
- **THEN** the stage and its receipt stay, the receipt keeps its original fields and records `sweep_refusals` and the last refusal's cause, OS error and descendant, nothing is deleted on uncertainty, and the sweep does not retry

#### Scenario: Last receipt collected
- **WHEN** a sweep collects the last receipt and the receipts folder holds only an empty `staging` directory
- **THEN** the sweep attempts one checked removal of the receipts folder, and a refusal leaves it for the next sweep and is reported to diagnostics

#### Scenario: Staging record kept
- **WHEN** a sweep leaves no receipt but the receipts folder's `staging` directory still holds a record
- **THEN** the receipts folder and its `staging` directory are kept

#### Scenario: Failed lock attempt on a warm open
- **WHEN** a warm open finds a receipt and its non-blocking lock attempt fails with an error
- **THEN** one diagnostics record reports the skipped sweep and its cause, and nothing on disk changes
