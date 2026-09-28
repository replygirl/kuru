# Tasks

## 1. Disable implicit exec installs in CI

- [x] 1.1 Set `MISE_EXEC_AUTO_INSTALL: "false"` in the workflow-level env of ci, quality, native-tests, bundle-build and release, and verify each file has exactly one such line
- [x] 1.2 Document the setting and the Windows shim mechanism in docs/development.md Shared build cache, and verify `docs:check` passes
- [x] 1.3 Verify with lint:tooling, format:check, cospec validate --strict and cospec:managed:check, and confirm `mise.toml` and `mise.lock` are unchanged

## 2. Prevent the class: no CI fetch a job does not use or the run already verified

- [x] 2.1 Add the workflow rules to the delivery repository check (mise opt-out at workflow level only, `install_args` on every mise-action step, no argument-less `mise install`, apt fetches restricted to a named list, offline bundle preparation in every partition job) with tests that reject each incident's pre-fix text and accept the fixed workflows
- [x] 2.2 Restrict the secret-store fixture apt step in native-tests and release to the runner image's Ubuntu archive list
- [x] 2.3 Fetch and verify the pinned upstream engine archives once per CI run in a `bundle-inputs` job and import them offline in every native-tests and native-memory partition
- [x] 2.4 Document the class rule and the check's coverage in docs/development.md
- [x] 2.5 Verify with the delivery test, lint and typecheck tasks, the repository check against the worktree and the pre-fix workflows, lint:tooling, format:check, docs:check, cospec validate --strict and cospec:managed:check
