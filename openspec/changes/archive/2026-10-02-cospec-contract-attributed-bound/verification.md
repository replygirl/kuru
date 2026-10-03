# Verification

## 1. A cospec call past its bound names itself and its phase [critical]

- [x] 1.1 @regression (agent) `mise run //packages/kuru-delivery:test`, test `a_call_that_cannot_finish_names_its_label_in_the_failure` (the helper run against `kuru-delivery-fixture never-finish` with a 2 s bound) -> the failure contains `standalone cospec never-finish fixture failed after ` and `tool timed out`, at the calling line. Before this change the helper took no label and its only failure text was `standalone cospec timed out: Elapsed(())` (jobs 110405506019 and 110887923249), which this assertion rejects. Observed 2026-10-02, macOS arm64: passes; the failure reads `standalone cospec never-finish fixture failed after 2005 ms: tool timed out`, located at the calling line (`cospec_contract.rs:113`).
- [~] 1.2 @integration (agent) the same test on this PR's windows-latest coverage legs, expecting the failure additionally contains `read native stdout/stderr after `, `arguments=["never-finish"]` and `samples=[`. -> defer: Windows-only assertions run on this PR's windows-latest and windows-11-arm legs; not runnable on this macOS host.

## 2. The contract keeps every gate and reports each call

- [x] 2.1 @integration (agent) `mise run //packages/kuru-delivery:test` with the pinned cospec -> `standalone_cospec_emits_one_document_and_preserves_every_gate` passes with unchanged gate assertions, and without `--nocapture` the output shows one `cospec <label>: <ms> ms` line for each of the nine calls plus the doctor `openspec-resolve` message. Observed 2026-10-02, macOS arm64: task exits 0; nine lines from 76 ms (`apply soft-blocked`) to 560 ms (`init`), doctor `no project @fission-ai/openspec; wrapped calls use the embedded pinned 1.13.1`.
- [~] 2.2 @runtime (agent) this PR's Windows coverage partition that runs the contract test, expecting its job log shows the nine per-call lines on a passing run. -> defer: needs this PR's CI; record the Windows per-call times in the roadmap notes.

## 3. First execution is provisioning

- [x] 3.1 @integration (agent) `mise run //packages/kuru-delivery:test`, guard `windows_on_arm_partitions_are_uninstrumented_behavioral_evidence_with_an_imported_engine` -> the step after the shard's mise-action step is exactly `Execute the installed cospec once` running `time cospec --version`; `mise run lint:tooling` accepts the workflow. Observed 2026-10-02, macOS arm64: both exit 0.
- [~] 3.2 @runtime (agent) this PR's native-tests shard jobs on every OS, expecting the step prints the pinned cospec version and its elapsed time. -> defer: needs this PR's CI.
