## Why

Kuru currently accepts user and ancestor TOML, a manually selected `--config` file, and a few fixed CLI flags. Projects need a deliberate local override and an externally provisioned policy without letting repository content masquerade as caller authority or override enforced constraints. Generic typed `-c` values also need to enter the same immutable, provenance-aware snapshot that workspace trust reviews.

## What Changes

- Discover `.kuru/config.local.toml` at the canonical project root as user-local authority outside the repository trust manifest. Reject it if Git tracks it; an unavailable or ambiguous Git index cannot establish the file's local provenance. Existing `--config` remains a deliberate explicit input.
- Add an externally provisioned managed layer with ordinary preferences and exact-value constraints over supported configuration leaves. Final effective values must satisfy the constraints after every local, saved or CLI override.
- Add repeatable typed `-c key=value` overrides for supported configuration leaves, with strict key/type validation and final-leaf command-line provenance.
- Expose the resolved layer order and managed constraint diagnostics through CLI and documentation while keeping credentials and sensitive values out of errors and trust output.

## Capabilities

### New Capabilities

### Modified Capabilities

- `configuration-schema`: Describe managed constraints and typed invocation overlays in the native parser and published schema.
- `workspace-trust`: Distinguish verified untracked local authority from repository-origin authority in a single immutable snapshot and preserve exact-root review.
- `chat-harness`: Apply the documented configuration order and reject values violating managed constraints before activation.

## Impact

Changes affect `packages/kuru-core` configuration and provenance APIs, `apps/kuru-tui` CLI, configuration docs and the published schema. Existing configuration files remain valid. The new managed path is outside repository discovery; no migration is required.

## Surfaces

- [x] interactive — CLI configuration and error UX
- [ ] deploy — no runtime topology or workflow change
- [x] integration — published configuration schema
- [x] agent-behavior — effective provider, tool and prompt configuration can change
