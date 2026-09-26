## 1. Native memory on Windows on Arm [critical]

- [ ] 1.1 @runtime (agent) `coverage:shard` `memory` and `runtime` shards (post-PR4b uniform five-shard job) on `windows-11-arm` with the PR6a engine imported through `bundle:prepare --archive --offline` -> both shards green uninstrumented (design Open Question 7, lead ruling 1), receipts record host triple `aarch64-pc-windows-msvc` and name the runs as arm64 behavioral evidence never counted toward the 90% gate, run id recorded here
- [ ] 1.2 @e2e (agent) `//apps/kuru-tui:test:embedded-runtime` on `windows-11-arm` with a cold offline engine cache -> install, first conversation, resume and update pass without any download

## 2. Owned process cleanup on Windows on Arm [critical]

- [ ] 2.1 @runtime (agent) `ci.yml native-platform` arm64 leg -> `kuru-platform` tests pass uninstrumented (lead ruling 1) and are recorded as separately named arm64 behavioral evidence outside the 90% gate, which the instrumented x64 and Unix runs keep enforcing; the arm64 coverage upload is held until a pinned toolchain carries the rust-lang/rust#150123 fix
- [ ] 2.2 @integration (agent) `connectors-core-platform` shard on `windows-11-arm` -> supervisor reap-before-release and owned-process tests pass natively

## 3. Native terminal behavior on Windows on Arm [critical]

- [ ] 3.1 @e2e (agent) `application` shard on `windows-11-arm` -> real ConPTY session tests pass with completed-frame synchronization

## 4. Installation on Windows on Arm [critical]

- [ ] 4.1 @e2e (agent) PR5's per-OS install/update job on `windows-11-arm`: `mise run install`, `bundle:verify-native-build`, `verify:windows-imports` -> source install completes, missing and corrupt-mirror builds fail at the memory build-script boundary, the shipping `kuru.exe` and embedded `dolt.exe` are PE ARM64 with OS-only imports
- [ ] 4.2 @integration (agent) bootstrap tests in the `delivery-archive` shard on both Windows runners -> the bootstrap selects the runner's native target while `PROCESSOR_ARCHITECTURE=AMD64` is injected into a native shell (proving the environment is ignored), rejects a mismatched explicit `-Target` with a message naming the native target, and requires the per-target PE machine; if step zero finds an x64 PowerShell host, a case launched from that emulated process also selects `aarch64-pc-windows-msvc`, otherwise the absence is recorded here
- [ ] 4.3 @unit (agent) `targets.rs` test after the catalog entry -> `for_platform("windows","aarch64")` yields `kuru.exe`/`zip`; `x86_64-pc-windows-gnu` still rejected
- [ ] 4.4 @integration (agent) bootstrap recovery case on both Windows runners -> an interrupted update whose receipt names an x64 helper (`x86_64-pc-windows-msvc-<sha>.exe`, PE machine `0x8664`) is recovered, and a helper whose name and PE machine disagree is rejected

## 5. Update on Windows on Arm [critical]

- [ ] 5.1 @e2e (agent) `verify-staged` leg `{windows-11-arm, aarch64-pc-windows-msvc}` (mise-route, mirroring `windows-latest`) on the first maintainer-authorized Release run after merge (release-time gate; recorded as an explicit post-merge deferral at archive, task 3.8) -> staged ZIP installs through loopback mise, offline demo persists and resumes, engine and licenses match the manifest, and the log carries either a predecessor-updater success or `no predecessor for aarch64-pc-windows-msvc: inspected ...`
- [ ] 5.2 @unit (agent) `published.rs` selection tests with an in-memory manifest provider -> `predecessor_is_the_greatest_older_release_carrying_the_target`, `predecessor_skips_releases_without_the_target_when_a_later_one_exists`, `no_predecessor_when_no_stable_release_carries_the_target`, `no_predecessor_requires_the_manifest_to_agree`, `no_predecessor_follows_every_listing_page` pass on every host without network access; fetched `SHA256SUMS` are authenticated against their listed digests
- [ ] 5.3 @integration (agent) `test:previous-release-update` in PR5's per-OS install/update job in ordinary CI on `windows-latest`, `macos-latest` and `ubuntu-latest` with `GITHUB_TOKEN` from CI -> the actual previous release's updater installs the candidate; the `Release` branch of the new enum behaves as before; on the `windows-11-arm` leg, until a release carries `aarch64-pc-windows-msvc`, the log carries `no predecessor for aarch64-pc-windows-msvc: inspected ...`
- [ ] 5.4 @e2e (agent) per-OS install/update job `test:embedded-runtime` on `windows-11-arm` -> `packaged_install_and_update_preserve_complete_offline_memory` passes natively; this is the pre-merge update evidence for the support claim

## 6. x64 stays green through every generalization [critical]

- [ ] 6.1 @regression (agent) full x64 Windows CI (`windows-2025` before PR5, `windows-latest` after; native-platform, five coverage shards, collect, install/update) on every phase-2 commit -> green with no catalog or workflow change; fixtures derive `AMD64` from the native machine
- [ ] 6.2 @unit (agent) `release_workflow.rs` gate cases, in the phase-3 workflow commit (task 3.4) because the test reads the live `native-tests.yml` gate script as PR5 rewrites it around shard, collect and install -> `windows-latest` accept/reject cases unchanged, `windows-11-arm` accept and each reject case added, unknown Windows label rejected
- [ ] 6.3 @integration (agent) `//packages/kuru-delivery:test`, `//packages/kuru-memory:test`, `//apps/kuru-tui:test` on macOS arm64 and x64 Windows after phase 2 -> all pass; `cargo check --target aarch64-pc-windows-msvc` for `kuru-platform` passes via the new cross-check task
- [ ] 6.4 @integration (agent) staged mise fixture asset ids after the #106 rebase -> ids derive from `CATALOG` position, `SHA256SUMS` has an id past the catalog, all ids are unique, and the `api_fallback` scenario passes on x64 Windows

## 7. Post-publication verification per target

- [ ] 7.1 @runtime (agent) `verify-published-windows` arm64 leg on the first arm64 release (post-publication; recorded as an explicit post-merge deferral at archive, task 3.8) -> receipt `published-windows-aarch64-pc-windows-msvc-receipt.json` records `target`, PE machine ARM64 and confirmed cleanup; x64 receipt name unchanged apart from the target segment
- [ ] 7.2 @unit (agent) receipt schema tests -> a receipt without `target` or with a target differing from the installed PE machine is rejected
- [ ] 7.3 @unit (agent) built-asset notice checks after PR6a's v2 schema -> the staged and published verifiers reject a built asset missing, or with a digest-mismatched, ICU, LLVM runtime or mingw-w64 notice, accept an upstream asset without notices, and the arm64 published receipt records the verified notices

## 8. Tooling and lockfiles on `windows-11-arm`

- [ ] 8.1 @manual (agent) step-zero probe job output -> stock PowerShell 5.1 native machine is ARM64 under both shells; `VC.Tools.ARM64` and `dumpbin.exe` paths listed; `rustc --print host-tuple` is `aarch64-pc-windows-msvc`; `cospec --version` and `cog --version` results, whether an x64 PowerShell host launches, the runner image version and Visual Studio edition/version (README and `vswhere`) recorded because the label migrates to Visual Studio 2026 during 2026-09-21 to 2026-09-30 (actions/runner-images#14602), and the instrumented `cargo llvm-cov` smoke result recorded verbatim, including `llvm-profdata merge -sparse` stderr so rust-lang/rust#150123 is confirmed or ruled out on Rust 1.98.1 (recorded as the pinned toolchain's state; per design Open Question 7, lead ruling 1, the arm64 suites stay uninstrumented until a pinned toolchain carries the fix)
- [ ] 8.2 @integration (agent) `mise install` under `MISE_LOCKED=1` on `windows-11-arm` with the `windows-arm64` lock entries -> every job's `install_args` tool installs; the emulated x64 cospec asset is tried first, and only if cospec still does not run is the named `cospec_contract` exclusion present in the shard receipt with its reason and nothing else skipped; if cospec runs, the receipt records that and nothing is excluded (lead ruling 5)
- [ ] 8.3 @regression (agent) root `mise.toml` cospec `platforms.windows-arm64.asset_pattern` -> `mise lock` generates the `windows-arm64` entry naming `cospec-0.7.1-windows-x64.zip`, a second `mise lock` produces no diff, the cocogitto `windows-arm64` entry records the x64 asset aqua selected, and a deliberately non-matching `asset_pattern` fails resolution instead of falling back to autodetection

## 9. Documentation and support claim

- [ ] 9.1 @manual (human) review `docs/install.md`, `docs/release.md`, `docs/development.md` in the final phase-3 commit -> the Windows on Arm row and any "supported" wording appear only after groups 1 to 4 and row 5.4 are green on the branch (release-time rows 5.1 and 7.1 are post-merge gates that block `deploy-docs` and promotion); `docs:check` passes
- [ ] 9.2 @regression (agent) `cospec validate --strict` and `cospec:managed:check` before archive -> pass
