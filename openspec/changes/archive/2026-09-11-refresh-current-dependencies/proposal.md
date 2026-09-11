## Why

New stable dependency and tooling releases appeared after Kuru's pins were
verified. Refresh them to honor the project's current-version requirement while
preserving reproducible, package-owned tasks.

## What Changes

- Update the shared `toml` dependency to 1.1.6 and its resolved lockfile entries.
- Update standalone cospec to 0.7.1, verify its actual command and archive gates,
  and retain the existing vendored-entry compatibility fix if still required.
- Pin npm 12.0.2 within `apps/kuru-docs`, alongside the current Node 26.8.2,
  using mise's native tool and task ownership instead of a root JavaScript project.
- Regenerate affected managed integrations and tool locks through their owning
  commands, and document observed compatibility and checks.

## Impact

Workspace dependency manifests/locks, root and docs-app mise configuration,
cospec-managed files and contributor instructions may change. Application source,
tests, release/Pages topology and runtime-provider bundling are outside this
maintenance scope. The separately planned bundled-provider change owns the
Codex client pin and its actual five-target runtime acceptance.
