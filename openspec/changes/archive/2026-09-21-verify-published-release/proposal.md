## Why

Release acceptance verifies staged packages, but the existing public Windows download verifier is manual. Phase 2 requires an automatic check of the immutable published tag and ordinary installation path, reported separately from publication.

## What Changes

- Add a post-publication Windows job to `.github/workflows/release.yml` using the existing package-owned verifier and upload its cleanup-confirmed receipt.
- Update `docs/release.md`, `docs/verification.md` and `AGENTS.md` to distinguish pre-publication gates from post-publication availability verification and same-run recovery.

## Impact

No new credentials, application code, release assets or publication entrypoints. Staged acceptance and docs deployment still gate publication. The new job runs from the exact released SHA with read-only repository permissions; failure is visible on the release run and never mutates or unpublishes an existing release.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
