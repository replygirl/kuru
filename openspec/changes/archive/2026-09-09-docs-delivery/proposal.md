## Why

The user requests a GitHub Pages docs site and cospec-style manual releases.
Shared tooling and required CI must cover those new delivery paths.

## What Changes

- Pin current docs/release tools in their owning mise app/package and keep the docs npm dependency graph local to its app.
- Add docs build/link validation to the repository gate and hosted CI.
- Add reusable/manual GitHub Pages deployment from an exact validated main SHA.
- Document local tasks and deployment metadata without changing repo visibility.

## Impact

Root tooling, lockfiles, CI, Pages workflow and development documentation.
Deployment uses GitHub Pages/OIDC permissions; release credential setup is owned
by the coordinated dispatch-releases change. Rust coverage remains at 90%.

## Surfaces

- [ ] interactive
- [x] deploy
- [ ] integration
- [ ] agent-behavior
