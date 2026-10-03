# Tasks

## 1. Attributed bound in the cospec contract fixture

- [x] 1.1 Route every cospec call through `kuru_delivery::command::output` with the 30 s bound, a subcommand label and `#[track_caller]` consumers, and verify `standalone_cospec_emits_one_document_and_preserves_every_gate` passes with unchanged gate assertions
- [x] 1.2 Write each finished call's elapsed time and the doctor resolution evidence to the raw stderr handle and verify they appear in a run without `--nocapture`
- [x] 1.3 Add the regression test `a_call_that_cannot_finish_names_its_label_in_the_failure` with a cross-platform `never-finish` fixture mode and verify it passes, rejecting the unlabelled pre-change failure text

## 2. First execution in CI tool setup

- [x] 2.1 Add `time cospec --version` directly after the native-tests shard's mise-action step, pin it in `release_workflow.rs`, follow it in `repo_validation.rs`, and verify `mise run //packages/kuru-delivery:test` and `mise run lint:tooling` pass

## 3. Checks

- [x] 3.1 Run `mise run format:check`, `mise run lint`, `mise run lint:windows`, `mise run typecheck` and `mise run lint:tooling` and verify each exits 0
- [x] 3.2 Run `mise run cospec -- validate cospec-contract-attributed-bound --strict` and `apply --json` and verify the gate is clear
