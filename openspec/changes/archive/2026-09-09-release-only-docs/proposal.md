## Why

The standalone Documentation dispatch allows Pages to publish independently of a release, contrary to the requested cospec pattern. Documentation must be the final stage of the release that supplies its source commit.

## What Changes

- Remove `.github/workflows/pages.yml` and inline `build-docs` and `deploy-docs` in `.github/workflows/release.yml` after successful publication.
- Build from the exact bump SHA and keep Pages permissions scoped to those jobs.
- Correct release operations and canonical agent guidance: redeploy by rerunning the docs jobs of an existing release run.

## Impact

Release topology and contributor instructions change. App code, dependencies, credentials and required CI checks remain unchanged. Validation covers the workflow graph, Actionlint, the full repository gate and confirmation that the canceled manual run created no deployment; no release will be dispatched for validation.

## Integration contract

Follow cospec's release dependency graph: `build-docs` needs both `bump` and `publish`, checks out `needs.bump.outputs.sha`, and uploads the validated Pages artifact. `deploy-docs` needs `build-docs` and is the final publication stage. Default success conditions skip these jobs when a prerequisite fails. Preserve existing pinned actions, the configured project base path and native docs checks. Build permissions are contents/read and pages/read; deploy permissions are pages/write and id-token/write, scoped to the github-pages environment. Recovery reruns these jobs on the original release run.

## Surfaces

- [ ] interactive
- [x] deploy
- [x] integration
- [ ] agent-behavior
