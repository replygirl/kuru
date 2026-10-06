# Proposal

## Why

Users currently have to combine several commands and documentation pages to distinguish sign-in state, configuration problems, project-memory health, and a damaged embedded engine. Troubleshooting that uncertainty can accidentally cross into network access, workspace authority, service startup, engine provisioning, or data repair unless the diagnostic boundary is explicit. The user-facing documentation also needs a concise account of local data locations, retention, the bundled engine, and the limits of Kuru's shell and integrity checks.

## What Changes

- Add an explicit read-only `kuru doctor` command with stable, fixed-code human and JSON reports for ChatGPT subscription status, Responses API-key route presence/configuration, configuration validity, workspace-trust status, current-project memory inspection, and the embedded engine payload.
- Keep the two authentication routes independent. The doctor never refreshes or contacts a provider, never prints account/key material, and does not inspect a repository-selected API-key environment variable until the relevant workspace authority has been approved; otherwise that route is reported as unverified.
- Inspect memory structure without creating state, starting/electing a service, migrating, provisioning Dolt, repairing files, or acquiring lifetime authority. A live compatible owner may be queried through a bounded read-only attachment; deeper cold SQL health remains unverified.
- Add curated troubleshooting, FAQ, data-location/privacy, and threat-model pages; explain target-dependent download size and embedded-engine behavior, retention and no-expiry semantics, status-check limits, and that explicit shell execution uses the user's process authority rather than a sandbox.
- Add isolated synthetic fixtures for healthy, fresh, corrupt, signed-out, untrusted, separate-route, and non-activating outcomes.

## Capabilities

### New Capabilities
- `local-diagnostics`: provide bounded local diagnosis without granting authority, contacting providers, creating state, or repairing it.

### Modified Capabilities
- `public-documentation`: add accurate troubleshooting, FAQ, data-location/privacy, download-size, and threat-model content that reflects current behavior.

## Impact

Changes are limited to `apps/kuru-tui` doctor dispatch/reporting and tests, the smallest read-only inspection seam and deterministic fixtures in `packages/kuru-memory`, and curated `apps/kuru-docs` pages/navigation. No dependency, credential schema, memory schema, migration, startup behavior, network operation, shell execution, or workspace-authority grant is added. A cold memory check reports the actual limit rather than provisioning an engine to claim deeper health.

## Surfaces

- [x] interactive — a user invokes and reads a diagnostic CLI report
- [ ] deploy — CI/runtime topology remains unchanged
- [ ] integration — no external protocol contract changes
- [ ] agent-behavior — no prompts, tools, model routing, or agent output changes
