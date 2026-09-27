# Proposal

## Why

The only way to exercise the Release workflow end to end today is a real publication: every dispatch plans, stamps a signed version commit on main, builds, accepts, deploys Pages and publishes. A rehearsal mode lets the maintainer prove the planned version, native archives, notes, candidate assembly, staged acceptance on every OS and the docs build on the current main revision without creating a commit, tag, GitHub release or Pages deployment.

## What Changes

- `.github/workflows/release.yml`: a `mode` dispatch input (choice `rehearsal`/`publish`, default `rehearsal`) next to `bump`. `bump`, `deploy-docs`, `publish` and `verify-published-windows` carry `if: inputs.mode == 'publish'`; `verify` is skipped with `bump`. `build`, `notes`, `assemble-candidate`, `verify-staged` and `build-docs` keep their `needs` and gain one-line guards whose publish branch is literally `success()` (the implicit default) and whose rehearsal branch requires each real prerequisite to succeed and `bump`/`verify` to be skipped. In rehearsal these jobs check out `github.sha` (the plan's `base_sha`), and `build`/`verify-staged` stamp the planned version into their local working tree only (the tree the version commit would hold). `notes` requires an exact clean commit carrying the planned version, so in rehearsal it stamps the same tree and commits `Cargo.toml`/`Cargo.lock` locally on the runner; that commit is never pushed (no credentials are persisted and the job is read-only). `plan`'s credential check reads the release app key only in publish mode; rehearsal checks the variable and the notes secret. `plan` and `assemble-candidate` write the run-summary banner `REHEARSAL: no commit, tag, release or Pages deployment`. Rehearsals use their own concurrency group and run name. Artifacts keep their names.
- `packages/kuru-delivery/tests/release_workflow.rs`: existing assertions are kept; the three that forbade any job/step `if:` and pinned the staged checkout expression now pin the exact guards instead. New assertions cover the input, per-job guards, input references, secret and write-permission scope, local stamps, the banner, the concurrency group and checkout refs.
- `docs/release.md` (new Rehearsal section and dispatch table), `AGENTS.md` (the single-entrypoint sentence names the mode input) and the Windows arm64 status note in the archived `windows-arm64-bundle-input` dependents list.

## Impact

Publish-mode jobs, `needs`, permissions, secrets and step lists are unchanged; a rendered publish-mode projection of the new file is identical to main's. The default dispatch now rehearses, so a real release requires selecting `mode=publish`. A rehearsal still uses the read-only `github.token` and the `ANTHROPIC_API_KEY_COMMUNIQUE` notes secret, never `RELEASE_APP_PRIVATE_KEY`, and no rehearsal job holds `contents: write`, `pages: write` or `id-token: write`. No product code changes.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
