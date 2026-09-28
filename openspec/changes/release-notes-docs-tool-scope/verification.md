# Verification

## 1. The notes and build-docs jobs install every tool they run, by name, before its first use [critical]

- [ ] 1.1 @regression (agent) `release_notes_and_docs_jobs_install_every_tool_they_run_before_its_first_use` on origin/main's release.yml, then on this tree -> pending
- [ ] 1.2 @regression (agent) `release_tool_derivation_rejects_each_unscoped_install_it_replaced`: the old notes `setup`, another named install, no docs tool step, the docs tool step after first use, and no Rust in `install_args` -> pending
- [ ] 1.3 @integration (agent) notes commands rehearsed in a fresh mise data directory with automatic installation off -> pending
- [ ] 1.4 @integration (agent) build-docs commands rehearsed the same way, with and without the docs tool step -> pending
- [~] 1.5 @runtime (human) a real Release run on GitHub-hosted ubuntu-latest under mise 2026.9.4: `notes` and `build-docs` succeed and print no `installing N tools` line from `mise run` -> defer: needs an authorized release dispatch, which this change's author may not perform

## 2. release.yml has no exemption

- [ ] 2.1 @regression (agent) `kuru-delivery repo --root` over a `git archive` of origin/main, then over this tree -> pending
- [ ] 2.2 @regression (agent) `release_workflow_has_no_task_auto_install_exemption` -> pending

## 3. Every other job's environment is unchanged

- [ ] 3.1 @manual (agent) parse origin/main's and this tree's release.yml and compare each job's effective env -> pending

## 4. Repository checks

- [ ] 4.1 @regression (agent) delivery test, lint and typecheck, `lint:tooling`, `format:check`, `docs:check`, `cospec validate --strict`, `cospec:managed:check` -> pending
