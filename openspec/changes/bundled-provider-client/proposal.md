## Why

Installing Kuru must include the native client required by its default provider,
including supported login and app-server operation. Kuru currently defaults to
an external `codex` command, so a complete Dolt bundle alone still leaves first
use dependent on a separate installation. The user explicitly requires bundled
runtime dependencies and native Windows support before this follow-up ships.

## What Changes

- Embed the verified official Codex 0.154.0 single-executable gzip archive and its
  license/notice material in every ordinary Kuru executable on all five native
  targets established by `native-windows`.
- Resolve the bundled client by default for login, logout, status, model/effort
  discovery and app-server inference; retain an explicit external-client override.
- Prepare pinned inputs through package-owned native mise/Rust tooling; Cargo
  selects by `TARGET` and verifies local bytes without downloading. Runtime
  extraction is private, bounded and verified, with no first-use download.
- Preserve Codex-owned authentication and Kuru-owned inference/tool authority;
  bundling does not enable Codex native tools, inspect credential stores or add
  another authentication implementation.
- Verify actual installed and updated executables with empty runtime caches,
  isolated `CODEX_HOME`, no external developer tools and native Windows/macOS/Linux
  execution, including meaningful app-server and login command probes.

## Capabilities

### New Capabilities

- `bundled-provider-client`: complete default Codex distribution, audited target
  manifests, local build preparation, private extraction and native acceptance.

### Modified Capabilities

- `provider-tools`: supported Codex authentication and inference use the bundled
  client by default while preserving explicit overrides and dynamic capabilities.

## Impact

Affected surfaces include `packages/kuru-connectors` Codex launch/provisioning,
`packages/kuru-core` client configuration, `apps/kuru-tui` authentication commands,
package build scripts/mise tasks, `packages/kuru-delivery` shared build-input and
archive/install/update bounds, native CI/Release acceptance, and owning install,
authentication, development and security documentation. Existing credential and
memory data are not migrated. The documented default changes from PATH lookup to
the bundled client; explicit configured external clients retain their deliberate
override behavior. The audited payload is the unmodified official executable,
upstream LICENSE and NOTICE, without package metadata or optional native-tool
companions. The existing delivery preparation engine accepts an explicit Codex
manifest adapter and prepares exact compressed bytes; connectors retain strict
local build and runtime payload validation. No new shared crate or consumer build
dependency is needed. Codex uses 128 MiB compressed/320 MiB expanded ceilings,
complete delivery uses 256 MiB ceilings, and Dolt retains its separate 64/128 MiB
limits. Actual combined release/test-artifact measurements and native acceptance
remain mandatory; `native-windows` still blocks source implementation.

## Surfaces

- [x] interactive — login/status errors and first provider conversation
- [x] deploy — embedded payload, native build inputs, installers, updates and CI
- [x] integration — official Codex distribution, notices, app-server and auth
- [x] agent-behavior — preserve provider isolation and discovered model/effort behavior
