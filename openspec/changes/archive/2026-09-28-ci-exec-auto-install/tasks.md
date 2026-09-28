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

## 3. Close the review's gaps in the check

- [x] 3.1 Extend the workflow-level-only rule to `MISE_TASK_RUN_AUTO_INSTALL`, move ci.yml's job-level settings to its workflow level, and exempt release.yml `notes` and `build-docs` by name, each pinned by the SHA-256 of its whole parsed job with the workflow `env` and `defaults` it inherits, requiring every other release job that uses mise to opt out in its job env, with tests that fail when either job changes in any value, is renamed or the exemption goes stale
- [x] 3.2 Read mise's global options before the subcommand and drop the values of its value-taking options (`-C/--cd`, `-E/--env`, `-j/--jobs`, `--minimum-release-age`, `--shared`, from the v2026.9.4 CLI reference) before deciding that `mise install` names no tool, with the review's bypasses as rejecting cases
- [x] 3.3 Read a shell's `-c` string as its command, reject `add-apt-repository` without `-n`, and treat any matrix job that runs a partition task as a partition whatever its axis is called
- [x] 3.4 Correct docs/development.md and this proposal to state the live release exemption and its follow-up `release-notes-docs-tool-scope`, and verify with the same checks as 2.5
- [x] 3.5 Pass mise-action `install_args` through the run-step operand logic and require a tool; treat argument-less `mise upgrade` and `mise bootstrap` as bare installs (mise 2026.9.4 CLI reference); skip the option values of `sudo`, `env`, `exec`, `time`, `timeout` and `nice` and timeout's duration; require exactly one apt `sourcelist`, a last `sourceparts` of `/dev/null` and no `-c`, and reject `APT_CONFIG`, with the second review's variants as rejecting cases
- [x] 3.6 List every variant a text rule still accepts in docs/development.md and the PR body, and verify with the same checks as 2.5
