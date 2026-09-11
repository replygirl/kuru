## Context

The existing connector launches `config.codex_command`, whose default is
`codex`, for app-server inference. TUI authentication has both connector-owned
commands and a direct device-login launch; all must use one resolution policy.
`isolation_config` disables provider-native execution and optional capabilities,
and the runtime owns structured tool proposals. The existing Dolt bundle already
separates memory-owned target verification from a native delivery preparation
helper. `native-windows` must finish adopting the native process, private file
and running-executable update contracts before this integration is applied.

Official documentation describes the app-server handshake and supported account
operations, and leaves token persistence to Codex:
[app-server](https://learn.chatgpt.com/docs/app-server) and
[authentication](https://learn.chatgpt.com/docs/auth). The current upstream audit
selected Codex 0.154.0 at official tag `rust-v0.154.0`, source commit
`6b9826e3aa83b1a5947db50f4332cb9c65f1b340`, for the implementation pin. All five
official single-executable gzip archives were downloaded, their published hashes
verified, and their executable entries independently measured and hashed. Actual
macOS arm64 execution and source review support the current restricted Kuru
protocol. Other targets still require native acceptance after implementation.

## Goals / Non-Goals

**Goals:** complete default-provider installation with one consistent client
resolver, native ownership guarantees, independently verified provenance and
meaningful five-platform evidence. Keep provider policy in connectors, OS
mechanics in platform, and preparatory downloads in package-owned build tooling.

**Non-Goals:** other providers, Codex-native tools, a new authentication stack,
credential import, general MCP/client bundling, provider self-update, a package
manager, or implementing blocked native Windows work as part of this change.

## Decisions

### 1. Ownership follows the existing provider boundary

Use `kuru-connectors` ownership for the Codex manifest, local build verifier,
private runtime resolver/extractor and their mise tasks. `kuru-core` owns the
typed configuration; `kuru-platform` supplies process/file guarantees;
`kuru-delivery` prepares build inputs without depending on connectors or memory.

The shared boundary is the delivery helper CLI and its verified archive file,
not a Rust dependency from a consumer build script. Refactor delivery's existing
`bundle.rs` preparation loop into one private byte-oriented function:

```rust
struct ArchivePin { url: Url, compressed_bytes: u64, sha256: [u8; 32] }
struct CacheOptions { bundle_dir: PathBuf, archive: Option<PathBuf>, offline: bool }
async fn prepare_archive(pin: &ArchivePin, options: &CacheOptions) -> Result<PathBuf>;
```

Private typed Dolt and Codex manifest adapters select an `ArchivePin` only after
validating the requested target, schema, URL, digest and applicable size ceiling.
They reject unknown fields, duplicates and mismatched manifest kind. The common
engine owns bounded HTTP/local import, exact compressed length/hash checks,
stable cache locking, identity revalidation and private atomic publication. It
does **not** decompress, inspect tar/ZIP members, verify extracted executables
or certify their runtime compatibility. Those checks remain with each owning
runtime. Existing Dolt cancellation/corruption/concurrent-import tests must still
exercise this same engine; add Codex adapter and cross-kind cases rather than
copying another transport/cache implementation.

Keep the Dolt command and environment behavior backward compatible. Add
`bundle prepare --kind codex`; omitted `--kind` remains `dolt`. The Codex task is
`mise run //packages/kuru-connectors:bundle:prepare -- --target <target>` and runs
the native delivery binary with `--kind codex`. Its default manifest is
`packages/kuru-connectors/support/codex-assets.json`; the Dolt default remains
`packages/kuru-memory/support/dolt-assets.json`. Resolve CLI arguments before
environment defaults selected by kind: `KURU_CODEX_BUNDLE_TARGET/DIR/ARCHIVE/OFFLINE`
for Codex, the existing `KURU_DOLT_BUNDLE_*` for Dolt. An unselected namespace
cannot affect the invocation, including when both are set. Preserve existing
explicit `--manifest`, `--target`, `--bundle-dir`, `--archive` and `--offline`
semantics. Default cache stays `<workspace>/target/kuru-bundles`; permit that
inference only for these two known package manifest locations, otherwise require
an explicit absolute directory. The returned file is always
`<bundle-dir>/<archive_sha256>.archive`. No receipt or helper output substitutes
for the build script independently reopening and checking that file.

Both owning mise tasks capture the intended consumer target before clearing
`CARGO_BUILD_TARGET`, `RUSTFLAGS` and `CARGO_ENCODED_RUSTFLAGS` for helper bootstrap.
Explicit `--target host` wins over an inherited foreign target; preparing a
foreign archive must still compile and execute the helper for the host. Consumer
Cargo receives the intended target explicitly, and build scripts use only Cargo
`TARGET`. The preparation binary depends on neither consumer and has no bundle
build script. This keeps source/offline builds acyclic without a new shared
crate, cross-package source includes, a delivery build dependency, or a generic
runtime framework. Independent manifest interpretation at preparation and build
is intentional: the first checks its byte input contract; the latter is the
owner's authoritative payload policy, both using the same checked-in manifest.

### 2. Default resolution is bundled and explicit overrides stay deliberate

Represent an omitted client override distinctly from a configured command;
preserve deliberately supplied existing `codex_command` values. Resolve once
through a shared connector API used by provider startup and every TUI auth route.
An omitted override never searches PATH or downloads a fallback client. An
explicit missing or incompatible override fails rather than silently changing
the user's selected authentication client. Keep the cache outside credential,
memory and tool roots. Reject automatic PATH fallback because it recreates the
install prerequisite and can make login and inference use different clients.

### 3. Preserve the upstream executable and prove the selected payload

Use the official per-executable gzip assets, keeping their bytes and signing
identity unmodified. Each measured archive contains exactly one regular executable
(mode 0755): USTAR on macOS, GNU tar on Linux/Windows, without PAX or package
metadata. Extract that executable and retain separately pinned upstream LICENSE
and NOTICE. The official README documents this standalone installation route.
There is no custom archive repack, package metadata or empty package-layout
directory requirement. Gzip reuses the existing decoder; smaller zstd assets
would add unnecessary format support for this slice.

Source review and the actual no-metadata macOS probe establish that Kuru's disabled
native-tool path does not require code-mode-host, ripgrep, zsh, bubblewrap or the
Windows sandbox helpers. Those optional resources are excluded. In particular,
the full Linux package's zsh would introduce unused glibc/libtinfo dependencies.
The separately packaged app-server executable is also unsuitable because it
omits Kuru's existing CLI login route. Each target must still prove the selected
standalone executable natively. Future native-tool enablement must revisit the
inventory rather than assuming these excluded features work.

### 4. Verify local build input and extract through a bounded typed format

Record target-to-upstream mapping, archive bytes/digests, exact member metadata
and notices in one authoritative connector manifest at
`packages/kuru-connectors/support/codex-assets.json`. Schema version 1 has root
fields `schema_version`, `version`, `upstream_commit`, `assets`, and `notices`.
Each of exactly five assets records `target`, `upstream_target`, `url`, `entry`,
`tar_format` (`ustar` or `gnu`), `compressed_bytes`, `archive_sha256`,
`expanded_bytes`, `executable_bytes`, and `executable_sha256`. Each of exactly two
notices records `name` (`LICENSE` or `NOTICE`), `url`, `bytes`, and `sha256`.
The measured catalog below supplies every pin; audit-only fields such as local
execution status and predicted combined size do not enter the runtime manifest.
Enforce known target mapping, exact release/source URL construction, native
format, one expected executable and fixed notice names; reject unknown fields.

Commit the unmodified notice files at
`packages/kuru-connectors/support/codex-notices/{LICENSE,NOTICE}`. They are small,
source-pinned inputs already present in a source checkout, so preparation does
not gain another notice downloader or repack step. The connector's `build.rs`
and `support/bundle.rs` verify their exact lengths/hashes and the prepared
archive, write those verified bytes and the selected catalog to `OUT_DIR`, and
register manifest, notice, archive and `KURU_CODEX_BUNDLE_DIR` rerun inputs.
Missing/corrupt input diagnostics name the connector-owned preparation command;
`KURU_CODEX_BUNDLE_DIR`, if present, must be absolute. Build preparation may
download only at an explicit native mise task, with pinned inputs or a validated
offline mirror; Cargo validates local data using `TARGET` and never networks.
Runtime verifies embedded bytes, the exact entry size/hash/type and private
staging before atomic activation, using native leases and identity checks. Reject
PAX, links, duplicate members and unexpected metadata instead of supporting the
unselected full-package format. Keep the existing Kuru release member allowlist
strict and the upstream gzip unmodified. Notices are separately verified
source-pinned bytes, not arbitrary archive members. Byte preparation never
claims these physical archive checks; tests pass correctly hashed malformed
fixtures through the owning decoder to establish that distinct boundary.

### 5. Authentication remains client-owned and transport behavior is stable

Use supported official `login`, device login, logout and login-status commands
and app-server initialization/model-list/thread/turn contracts. Preserve current
Codex home/account selection semantics; do not silently redirect existing users
to a new credential location. Tests always supply private fresh `CODEX_HOME` and
explicit environments. Kuru never reads or relocates auth files. Keep unknown
models/efforts and the inference-only configuration intact; the bundle does not
grant provider-native execution authority. Retain process-tree ownership and
bounded pipe cleanup through the native Windows API once that prerequisite lands.

### 6. Delivery is measured as a complete installed product

The current 128 MiB delivery ceiling cannot fit the selected payload: measured
macOS release Kuru with Dolt (52,167,840 bytes), the smallest Codex gzip (88,080,735)
and both notices (11,168) already total 140,259,743 bytes before added code/alignment.
The corresponding debug estimate is 178,838,071 bytes. Fix the implementation
ceilings as follows; none is a user-configurable override or inferred from an
untrusted response/header:

| Boundary | Bytes | Enforcement |
| --- | ---: | --- |
| Bundle manifest | 65,536 (64 KiB) | Checked regular read before JSON decoding |
| Each Codex notice / both notices | 65,536 / 131,072 | Fixed two names; exact recorded lengths/hashes remain authoritative |
| Codex compressed archive | 134,217,728 (128 MiB) | Delivery adapter, owner build verifier and runtime |
| Codex expanded tar / executable member | 335,544,320 (320 MiB) | Owner manifest policy and bounded runtime decoder |
| Outer release archive download | 268,435,456 (256 MiB) | Packager, updater and shell/PowerShell bootstrap |
| Outer expanded archive / Kuru executable | 268,435,456 (256 MiB) | Packager preflight and all native extraction paths, including headers, padding and allowed documentation |

Exact target lengths and hashes remain stricter than the ceilings. The largest
measured Codex archive is 99,510,903 bytes, leaving 34,706,825 bytes below its
compressed ceiling; the largest expanded tar is 298,178,560 bytes, leaving
37,365,760 below its expanded ceiling. Dolt's independent 64 MiB compressed and
128 MiB expanded limits remain unchanged in its adapter, verifier and runtime.
This choice retains gzip, exact official assets and finite rejection behavior;
raising a universal engine cap or using a first-run download is rejected.

The combined **input-only** sums (Dolt archive + Codex archive + 11,168 notice
bytes) are 129,224,578 / 139,543,991 / 132,529,782 / 142,964,084 / 139,560,748
bytes in the target table's order. They exclude Kuru code, symbols, alignment and
outer packaging. These and the macOS release/debug additions above are estimates,
not actual combined executables. Before completion, record for **each** target
the built release executable, the debug/instrumented executable actually used by
packaged tests, the resulting compressed release archive and its expanded total.
Record source SHA/profile and compare every measurement with its applicable cap.
A target exceeding a cap fails acceptance and requires a reviewed design change;
do not silently expand a constant, truncate notices, disable embedding, or replace
the actual package fixture with a smaller unrepresentative binary. Apply the
same outer bounds to native packaging/updating and shell/PowerShell bootstrap
checks, retaining checksummed over-limit rejection and unchanged-destination
regressions for both compression and expansion boundaries.
Native tests install/update actual release-built executables and then use empty
runtime caches and no external developer tools. Preserve existing release SHA,
checksums, strategy-only dispatch, recovery and final Pages topology. Do not add
a second installer or a first-run dependency download as a size workaround.

Existing installations cross the size-limit boundary through the same documented
bootstrap or mise installation route. The published v0.1.0 Unix updater enforces
128 MiB before installing a candidate; changing the new executable's constant
cannot change that old validator. The current pre-bundle Windows helper has the
same cap, but v0.1.0 did not publish a Windows executable. Document this version
boundary and verify the real old-to-new installer path, including preservation of
the old executable when its updater rejects an oversized candidate and retention
of conversation data across reinstallation. Do not claim that a new-to-new update
proves this transition. This uses the existing installer and does not require a
bridge release or authorize a release dispatch. Retained published-source
evidence: `/tmp/kuru-v0.1.0-archive.rs` and
`/tmp/kuru-current-release-inventory.json`.

The larger embedded image must also pass the actual Windows packaged and source
update paths within the existing ten-second acknowledgment budget. Preserve
independent hashes, file identities, durable receipts, cleanup and normal helper
profile collection. The measured development SHA-2 optimization supports this
work but does not predict the timing of the full Codex-bearing image; record
that native result separately.

## Operational surface

The client runs as a local owned subprocess; app-server uses the existing stdio
transport, with no new listener, daemon, container or service. Authentication
may use the official client's browser callback or device flow and online provider
traffic; Kuru does not replace those supported flows. Existing runtime parallel
and request/output bounds still apply, including native descendant cleanup.
Native targets are macOS arm64/x64, Linux GNU arm64/x64 (explicitly mapped to the
audited official musl client where appropriate), and Windows x64/MSVC supplied by
the hard prerequisite. The selected client version is 0.154.0; the five-target
native acceptance ledger remains incomplete.

Private client caches and build inputs are independent of credential stores and
tool roots. No new repository or provider secret is required for bundle checks:
native CI uses isolated CODEX_HOME and unauthenticated supported commands. Live
login/inference is explicitly separate and user-authorized; existing credentials
stay under Codex control. Installer failures distinguish invalid bundle, missing
override and unauthenticated provider without asking users to install npm/Node or
download a hidden fallback. Release publication and Pages remain in their existing
workflow; native smoke tests run locally on each runner before artifact upload.

## Integration contract

Connectors own client path resolution and Codex app-server request semantics;
the TUI delegates every auth route through that resolver. The transport remains
initialize then initialized, model/list pagination with string identifiers and
unfamiliar effort values preserved, and the existing restricted thread/turn
structured-output protocol. No model name is hard-coded by the bundle selector.
Use actual upstream command responses in native compatibility fixtures; fake
provider endpoints may establish replay/isolation but not live service success.

The completed audit is `/tmp/kuru-codex-bundle-audit.md`, with exact five-target
hashes/bytes and notice pins in `/tmp/kuru-codex-bundle-proof/catalog.json`.
Its exact selected measurements are retained below as planning evidence for the
future package manifest. URLs use the official release prefix
`https://github.com/openai/codex/releases/download/rust-v0.154.0/` followed by
`codex-<upstream-target>.tar.gz` (Windows: `.exe.tar.gz`). The sole entry is the
filename without `.tar.gz`; all entries are regular files with mode 0755.

| Kuru target | Upstream target | Gzip bytes | Executable bytes | Expanded tar bytes |
| --- | --- | ---: | ---: | ---: |
| aarch64-apple-darwin | aarch64-apple-darwin | 88080735 | 222655232 | 222657024 |
| x86_64-apple-darwin | x86_64-apple-darwin | 95929450 | 239700064 | 239702016 |
| aarch64-unknown-linux-gnu | aarch64-unknown-linux-musl | 91768360 | 227482840 | 227491840 |
| x86_64-unknown-linux-gnu | x86_64-unknown-linux-musl | 98981886 | 262858016 | 262860800 |
| x86_64-pc-windows-msvc | x86_64-pc-windows-msvc | 99510903 | 298169136 | 298178560 |

| Kuru target | Archive SHA-256 | Executable SHA-256 |
| --- | --- | --- |
| aarch64-apple-darwin | `344310a0a591c1b192e04feff304321a69907c9498baaac331ca7e16ebcef9d7` | `4f85982624b3898c8991cb80c0981b2aa71070e3537046c9a95950318a95afcc` |
| x86_64-apple-darwin | `1219c837d8f813b493a424c125c0038b5d9ca16279bc6d3fe6ce037a3e18a6e7` | `b0e26f09819c4b27f621853800c29f95ac5d526ad9b88ab641de4db86835718d` |
| aarch64-unknown-linux-gnu | `583b48df32804213bdcd338c2e5adb06b34340821fa757a726cc0a524fa33c27` | `9b7c1c7abdc26fc3c4f47c77656a8e9121def5483dbae830ef1ee561758448a9` |
| x86_64-unknown-linux-gnu | `d7e18b2597ae8f242f5f31ee9e90deef48dbc9edd634d9868fb6435d08c07f02` | `3188814c35471432d4123203e0eb38e5bddc60226e3d7ddf0e59e649ea140022` |
| x86_64-pc-windows-msvc | `4e96740782869faff9d424806d4419afd2ee51ea5ece6cec462912b7098497a1` | `be96b992178b1e467c225800da0d65f2c86d5eba1ef0b14632f65db381cbdfde` |

The strongest actual macOS proof uses the raw executable with no metadata or
companions, cleared environment, empty PATH and isolated HOME/CODEX_HOME. It
passes version/login help, initialize/model-list with pagination and dynamic
efforts, restricted ephemeral thread/start and successful EOF cleanup. The trace
is `/tmp/kuru-codex-bundle-proof/standalone-app-server.log`. No authenticated
inference was sent. Upstream websocket warmup attempted networking and failed DNS;
this is not a claim that provider operation never networks.

Pinned source supports the minimal path: `thread_manager.rs` selects
DisabledCodeModeSessionProvider unless host mode or forced fallback is enabled;
the host provider is lazy. Bubblewrap C builds only into a separate bin-only
target, with its digest embedded in Codex; the main executable's Rust launcher
opens, verifies and executes that external helper. Excluding the helper therefore
does not redistribute its C binary through this path. See the
[host gate](https://github.com/openai/codex/blob/6b9826e3aa83b1a5947db50f4332cb9c65f1b340/codex-rs/core/src/thread_manager.rs#L472),
[separate helper release](https://github.com/openai/codex/blob/6b9826e3aa83b1a5947db50f4332cb9c65f1b340/.github/workflows/rust-release.yml#L221)
and [external launcher](https://github.com/openai/codex/blob/6b9826e3aa83b1a5947db50f4332cb9c65f1b340/codex-rs/linux-sandbox/src/bundled_bwrap.rs#L28).

Retain exact source LICENSE (10,926 bytes; SHA-256
`d17f227e4df5da1600391338865ce0f3055211760a36688f816941d58232d8dc`)
and NOTICE (242 bytes; SHA-256
`9d71575ecfd9a843fc1677b0efb08053c6ba9fd686a0de1a6f5382fd3c220915`).
They are absent from the official archives and are pinned separately to the same
commit. This is the selected main executable's upstream notice inventory; adding
any optional component requires a new provenance review. The manifest maps Kuru
GNU Linux targets to the corresponding official musl clients explicitly.

## Risks / Trade-offs

- [Untested native target] → The official standalone path and macOS behavior are
  verified; run actual startup/discovery/login-help and cleanup on every remaining
  target before claiming support.
- [Optional-feature dependency drift] → Keep native tools disabled and revisit
  the inventory before enabling them; excluded Linux zsh requires GLIBC 2.38 and
  libtinfo, while the inspected main Linux executable is static-PIE.
- [Large executable] → Audit actual compressed/expanded sizes and measure release
  binaries; retain finite bounds and adversarial decoder/install tests.
- [Missing notices] → Embed and retain the separately pinned source LICENSE and
  NOTICE through cache extraction/install/update; test exact bytes and hashes.
- [Version/protocol drift] → Keep release identity and source mapping exact and
  execute Kuru's existing restricted transport fixture against the proposed pin.
- [Credential exposure] → Preserve client-owned auth, isolated test homes,
  explicit environment handling and redacted diagnostics; never inspect stores.
- [Cross-platform assumption] → Native initialize/login-help/model discovery is
  required on all five targets. Neither a macOS probe nor Windows cross-compiling
  establishes other platforms' runtime support.

## Planning decision status

Both former pre-implementation design questions are resolved: decision 1 fixes
the byte-only delivery interface and independent connector verifier; decision 6
fixes the finite per-runtime and outer delivery budgets. The audited asset pin,
standalone format and notices remain unchanged. No design choice requires a
consumer dependency cycle or a new runtime framework.

Actual combined release/test-artifact bounds on all five targets, native tests
on the four unexecuted targets and live authenticated behavior remain required
implementation acceptance. They are neither completed by this review nor
pre-implementation questions that require building blocked product code.
`native-windows` remains the hard dependency: source implementation still
requires its completion and an actual clear cospec apply.
