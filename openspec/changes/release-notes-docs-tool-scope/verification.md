# Verification

## 1. The notes and build-docs jobs install every tool they run, by name, before its first use [critical]

- [x] 1.1 @regression (agent) `release_notes_and_docs_jobs_install_every_tool_they_run_before_its_first_use` on origin/main's release.yml (686eb16b), then on this tree -> origin/main fails with 7 problems: no workflow-level `MISE_TASK_RUN_AUTO_INSTALL`; notes `setup` installs every configured tool; the notes step needs Cocogitto and Communiqué before any named install; build-docs step 3 needs node for `check:toolchain` and node and npm for `setup` before any named install. This tree passes and derives notes = {rust, Cocogitto, Communiqué} and build-docs = {rust, node, npm}
- [x] 1.2 @regression (agent) `release_tool_derivation_rejects_each_unscoped_install_it_replaced`: the old notes `setup`, a named install of other tools (`setup:advisories`), no docs tool step, the docs tool step moved after the first use, and no Rust in build-docs `install_args` -> each yields its exact findings; test passes
- [x] 1.3 @integration (agent) notes commands rehearsed with local mise 2026.9.13 (CI pins 2026.9.4) in a fresh data directory, automatic installation off, `MISE_LOCKED=1`, `KURU_MBX=0`, no keys, network cut at a dead local proxy for the notes command -> the old `setup` would install 11 tools (`install --dry-run --include-task-tools`); the named step installed exactly Cocogitto 7.0.0 and Communiqué 1.4.2; `release:tool` built kuru-release and stamped a scratch candidate; `notes` reached Communiqué, which failed at the cut network ("Communiqué failed (exit status: 1)"). Without the named step the same command failed with "Communiqué execution failed ... No such file or directory" and nothing was installed
- [x] 1.4 @integration (agent) build-docs commands rehearsed the same way, with and without the docs tool step -> with it: `setup:tools`, `setup`, `docs:build` and `kuru-delivery docs --base /kuru/` exit 0 ("Public docs artifacts, local links and anchors passed (/kuru/)"). Without it: `mise run //apps/kuru-docs:setup` installed node and npm through its dependency and then failed "sh: node: command not found" (exit 127). checkout, configure-pages and upload-pages-artifact are GitHub actions and did not run
- [~] 1.5 @runtime (human) a real Release run on GitHub-hosted ubuntu-latest under mise 2026.9.4: `notes` and `build-docs` succeed and print no `installing N tools` line from `mise run` -> defer: needs an authorized release dispatch, which this change's author may not perform

## 2. release.yml has no exemption

- [x] 2.1 @regression (agent) `kuru-delivery repo --root` over a `git archive` of origin/main, then over this tree -> origin/main exit 1 with 9 release.yml findings (8 job-level overrides and the missing workflow-level setting); this tree exit 0, "Repository metadata invariants passed."
- [x] 2.2 @regression (agent) `release_workflow_has_no_task_auto_install_exemption`: removing the workflow-level setting, the old per-job arrangement, job-level settings in notes and build-docs, a build-docs step override and a bare notes install -> each rejected with its exact finding; test passes

## 3. Every other job's environment is unchanged

- [x] 3.1 @manual (agent) parse origin/main's and this tree's release.yml and compare each job's effective env -> tests, plan, bump, build, assemble-candidate, verify-staged, publish and verify-published-windows identical; notes and build-docs gain only `MISE_TASK_RUN_AUTO_INSTALL=false`; deploy-docs gains it but runs no mise; source-quality, verify and dolt-windows-arm64 call reusable workflows, which caller env does not reach; no step overrides it

## 4. Repository checks

- [x] 4.1 @regression (agent) delivery test, lint and typecheck, `lint:tooling`, `format:check`, `docs:check`, `cospec validate --strict`, `cospec:managed:check` on macOS arm64 -> delivery test exit 0 (328 passed, 0 failed, 1 ignored: the opt-in previous-release update); every other check exit 0; no lockfile changed. The 90% workspace line gate was not measured locally
