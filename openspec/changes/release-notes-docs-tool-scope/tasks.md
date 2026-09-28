# Tasks

## 1. Scope the release jobs' tool installation

- [ ] 1.1 Set `MISE_TASK_RUN_AUTO_INSTALL: "false"` at release.yml workflow level, remove the job-level copies, add the docs tool step to `build-docs` and the named notes tool install to `notes`, and verify each job's effective environment against origin/main
- [ ] 1.2 Remove the workflow check's exemption and replace its tests with a no-exemption test, and verify the check rejects origin/main's release.yml and passes this tree
- [ ] 1.3 Add the release tool derivation tests for `notes` and `build-docs`, and verify they fail on origin/main's release.yml and pass on this tree
- [ ] 1.4 Rehearse each job's commands locally in a fresh mise data directory with automatic installation off, and record which commands ran, which could not and why
- [ ] 1.5 Update docs/development.md and docs/release.md, and verify `docs:check` and `format:check` pass
- [ ] 1.6 Run delivery test, lint and typecheck, `lint:tooling`, `cospec validate --strict` and `cospec:managed:check`, and record the results
