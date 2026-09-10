## ADDED Requirements

### Requirement: Complete default provider installation

Every ordinary Kuru executable SHALL contain the official native Codex executable
and every native resource required by Kuru's supported login and restricted
app-server inference paths, including applicable license and notice material.
The five supported native targets MUST work without npm, Node, a separately
installed Codex client or any first-use client download. The bundle MUST NOT
enable Codex native tool execution or imply that unrelated optional tools are
bundled. Required OS facilities and online authentication/inference remain
distinct from runtime provisioning.

#### Scenario: First default-provider use
- **WHEN** a freshly installed executable starts with an empty client cache and no external Codex or developer tools on PATH
- **THEN** it provisions its embedded client locally and can run supported login help, app-server initialization and model discovery without downloading a runtime.

#### Scenario: Payload dependency audit
- **WHEN** a proposed native payload requires an absent non-OS helper or dynamic library on a supported target
- **THEN** acceptance fails until that dependency is included and verified or source and native evidence establish that the required Kuru path does not use it.

### Requirement: Authoritative target inputs

The owning package SHALL maintain one exact manifest mapping each Kuru build
target to audited official client assets, upstream version and source identity,
compressed and expanded byte bounds, required members, per-member SHA-256 values
and notice provenance. The selected version MUST pass actual source/protocol
compatibility and native dependency checks before becoming the pin. A native
package-owned mise task SHALL prepare verified local build inputs; Cargo MUST
select the exact `TARGET`, refuse absent, corrupt, mismatched or unsupported
inputs and perform no network operation. The delivery helper SHALL share only
bounded compressed-byte preparation between explicit Dolt and Codex manifest
adapters; each runtime SHALL retain its independent strict payload policy.
The helper MUST NOT depend on either consumer or claim that byte preparation
has validated physical archive members. Exact source-pinned Codex notices SHALL
be checked-in owning-package inputs verified locally by the build script.

#### Scenario: Cross-target input mismatch
- **WHEN** Cargo is given a valid archive for a different target or a same-size corrupt input
- **THEN** compilation fails with target and input diagnostics rather than using a host fallback or emitting an unbundled executable.

#### Scenario: Offline prepared build
- **WHEN** the required verified input and notices are already present locally
- **THEN** the owning mise/Cargo build completes without fetching a client and embeds those exact verified bytes.

#### Scenario: Compatible preparation and independent overrides
- **WHEN** existing Dolt preparation runs without a kind flag, or Codex preparation runs with `--kind codex`, while both runtime-specific environment namespaces are present
- **THEN** the existing Dolt command defaults remain compatible, each invocation uses only its selected namespace beneath explicit CLI arguments, and neither changes the other's target, archive, cache or offline behavior.

#### Scenario: Shared bytes do not certify archive members
- **WHEN** a correctly hashed malformed archive has been prepared by the shared byte engine
- **THEN** the owning runtime still rejects its invalid structure and no prepared-file receipt or helper success bypasses that check.

### Requirement: Strict private runtime extraction

Runtime client provisioning MUST use embedded bytes only, validate archive
structure with a typed bounded decoder, verify each selected member and its
identity, and publish a complete private cache only after verification. The
decoder MUST accept only the exact single regular executable entry in the
audited USTAR/GNU gzip archives, with separately pinned notice bytes. It MUST
reject PAX or other unexpected metadata, traversal, absolute or device names,
duplicate entries, links/reparse points, unexpected members and expanded-size
overflow. Cache reuse MUST verify completeness and ownership. Cancellation,
concurrent first use and failed extraction MUST preserve existing verified
clients and never leave an executable selected from a partial cache.

#### Scenario: Malicious or corrupt embedded input
- **WHEN** an archive member exceeds its bound, aliases another file or fails its digest check
- **THEN** no candidate is activated and a previous verified client remains usable.

#### Scenario: Concurrent first extraction
- **WHEN** two native processes prepare the same cold cache and one is cancelled
- **THEN** activation remains atomic, surviving callers receive one verified payload, and no owned process outlives the files it still uses.

### Requirement: Complete delivery and native evidence

Direct, mise and source installation and native self-update SHALL preserve the
complete embedded Dolt and Codex payload. Codex SHALL use independent ceilings
of 128 MiB compressed and 320 MiB expanded, while Dolt retains its 64 MiB
compressed and 128 MiB expanded ceilings. Every outer release archive download,
expanded release archive and Kuru executable SHALL be bounded to 256 MiB; exact
manifest lengths and hashes MUST remain stricter than these ceilings. Actual
combined release and packaged-test executables and their compressed/expanded
archives MUST be measured on all five native targets before acceptance. An
over-limit target MUST fail rather than silently expanding a ceiling, omitting
payload or replacing a representative test binary. Larger payloads MUST NOT
bypass checksum, typed archive or atomic replacement protections. Every supported
native CI target SHALL execute the installed or release-built executable with
empty caches and isolated test state. Tests MUST distinguish real offline
initialize/model-list/login-help evidence from an actual online login or
authenticated inference result.

#### Scenario: Install and update from release assets
- **WHEN** a native executable is installed and subsequently updated from verified release archives
- **THEN** both installations can provision their required clients from independent empty caches and preserve complete notices without a compiler or first-use runtime download.

#### Scenario: Complete artifact exceeds a finite ceiling
- **WHEN** a verified download, expanded archive or native executable exceeds its applicable ceiling, including allowed documentation and archive overhead
- **THEN** packaging or installation fails before replacing the installed executable; arithmetic on embedded input sizes does not count as a passing native artifact measurement.

#### Scenario: Unsupported claim from a fixture
- **WHEN** CI succeeds using an isolated unauthenticated client or a local fake provider endpoint
- **THEN** the evidence records that boundary and does not claim a completed live login or real provider inference.
