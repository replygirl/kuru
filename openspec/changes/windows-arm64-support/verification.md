## 1. Native memory on Windows on Arm [critical]

- [ ] 1.1 @runtime (agent) `windows-coverage` `memory` and `runtime` shards on `windows-11-arm` with the PR6a engine imported through `bundle:prepare --archive --offline` -> both shards green, receipts record host triple `aarch64-pc-windows-msvc`, run id recorded here
- [ ] 1.2 @e2e (agent) `//apps/kuru-tui:test:embedded-runtime` on `windows-11-arm` with a cold offline engine cache -> install, first conversation, resume and update pass without any download

## 2. Owned process cleanup on Windows on Arm [critical]

- [ ] 2.1 @runtime (agent) `ci.yml native-platform` arm64 leg -> `kuru-platform` coverage at or above 90% lines, artifact `coverage-native-platform-aarch64-pc-windows-msvc` uploaded
- [ ] 2.2 @integration (agent) `connectors-core-platform` shard on `windows-11-arm` -> supervisor reap-before-release and owned-process tests pass natively

## 3. Native terminal behavior on Windows on Arm [critical]

- [ ] 3.1 @e2e (agent) `application` shard on `windows-11-arm` -> real ConPTY session tests pass with completed-frame synchronization

## 4. Installation on Windows on Arm [critical]

- [ ] 4.1 @e2e (agent) `windows-install` on `windows-11-arm`: `mise run install`, `bundle:verify-native-build`, `verify:windows-imports` -> source install completes, missing and corrupt-mirror builds fail at the memory build-script boundary, the shipping `kuru.exe` and embedded `dolt.exe` are PE ARM64 with OS-only imports
- [ ] 4.2 @integration (agent) bootstrap tests in the `delivery-archive` shard on both Windows runners -> the bootstrap selects the runner's native target while `PROCESSOR_ARCHITECTURE=AMD64` is injected, rejects a mismatched explicit `-Target`, and requires the per-target PE machine
- [ ] 4.3 @unit (agent) `targets.rs` test after the catalog entry -> `for_platform("windows","aarch64")` yields `kuru.exe`/`zip`; `x86_64-pc-windows-gnu` still rejected

## 5. Update on Windows on Arm [critical]

- [ ] 5.1 @e2e (agent) `verify-staged-windows` `{windows-11-arm, aarch64-pc-windows-msvc}` on a real Release run -> staged ZIP installs through loopback mise, offline demo persists and resumes, engine and licenses match the manifest, and the log carries either a predecessor-updater success or `no predecessor for aarch64-pc-windows-msvc: inspected ...`
- [ ] 5.2 @unit (agent) `published.rs` selection tests -> `predecessor_is_the_greatest_older_release_carrying_the_target`, `predecessor_skips_releases_without_the_target_when_a_later_one_exists`, `no_predecessor_when_no_stable_release_carries_the_target`, `no_predecessor_requires_the_manifest_to_agree` pass on every host
- [ ] 5.3 @integration (agent) `test:previous-release-update` on x64 Windows, macOS arm64 and Linux with a read `GITHUB_TOKEN` -> the actual previous release's updater installs the candidate; the `Release` branch of the new enum behaves as before

## 6. x64 stays green through every generalization [critical]

- [ ] 6.1 @regression (agent) full x64 `windows-2025` CI (native-platform, five coverage shards, report, install) on every phase-2 commit -> green with no catalog or workflow change; fixtures derive `AMD64` from the native machine
- [ ] 6.2 @unit (agent) `release_workflow.rs` gate cases -> `windows-2025` accept/reject cases unchanged, `windows-11-arm` accept and each reject case added, unknown Windows label rejected
- [ ] 6.3 @integration (agent) `//packages/kuru-delivery:test`, `//packages/kuru-memory:test`, `//apps/kuru-tui:test` on macOS arm64 and x64 Windows after phase 2 -> all pass; `cargo check --target aarch64-pc-windows-msvc` for `kuru-platform` passes via the new cross-check task

## 7. Post-publication verification per target

- [ ] 7.1 @runtime (agent) `verify-published-windows` arm64 leg on the first arm64 release -> receipt `published-windows-aarch64-pc-windows-msvc-receipt.json` records `target`, PE machine ARM64 and confirmed cleanup; x64 receipt name unchanged apart from the target segment
- [ ] 7.2 @unit (agent) receipt schema tests -> a receipt without `target` or with a target differing from the installed PE machine is rejected

## 8. Tooling and lockfiles on `windows-11-arm`

- [ ] 8.1 @manual (agent) step-zero probe job output -> stock PowerShell 5.1 native machine is ARM64 under both shells; `VC.Tools.ARM64` and `dumpbin.exe` paths listed; `rustc --print host-tuple` is `aarch64-pc-windows-msvc`; `cospec --version` and `cog --version` results recorded verbatim
- [ ] 8.2 @integration (agent) `mise install` under `MISE_LOCKED=1` on `windows-11-arm` with the `windows-arm64` lock entries -> every job's `install_args` tool installs; if cospec does not, the named `cospec_contract` exclusion is present in the shard receipt with its reason and nothing else is skipped

## 9. Documentation and support claim

- [ ] 9.1 @manual (human) review `docs/install.md`, `docs/release.md`, `docs/development.md` in the final phase-3 commit -> the Windows on Arm row and any "supported" wording appear only after groups 1 to 5 are green; `docs:check` passes
- [ ] 9.2 @regression (agent) `cospec validate --strict` and `cospec:managed:check` before archive -> pass
