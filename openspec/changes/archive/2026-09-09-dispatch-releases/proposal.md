## Why

Maintainers currently edit versions and push a tag before the release workflow can run. Kuru needs one deliberate dispatch that computes a conventional version, validates an immutable revision, and publishes complete native archives and release notes using the established cospec pattern.

## What Changes

- Replace tag-triggered publication with manual auto/major/minor/patch dispatch and explicit retry of an existing version.
- Stamp workspace versions, create signed GitHub API commits with optimistic concurrency, and retain immutable tags.
- Generate Communiqué notes with current configuration and first-release context, assemble verified complete assets, and publish a draft only after all checks succeed.
- Invoke the independently maintained Pages workflow on the exact released commit.
- Document the app, model credential, narrow ruleset exception, failure recovery, and private installation prerequisites.

## Capabilities

### New Capabilities

- `release-automation`: conventional version calculation, signed immutable release preparation, complete publication and recovery.

### Modified Capabilities

None.

## Impact

Changes .github/workflows/release.yml, cog.toml, communique.toml, native release package/tests, mise tooling and docs/release.md. No runtime APIs or stored user data change. Hosted release dispatch and settings changes remain separate authorized actions.

## Surfaces

- [ ] interactive — no chat UI change
- [x] deploy — GitHub Actions and release publication
- [x] integration — GitHub API, Cocogitto and Communiqué
- [x] agent-behavior — release notes model instructions
