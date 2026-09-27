## 1. Native memory on Windows on Arm [critical]

- [ ] 1.1 @runtime (agent) `coverage:shard` `memory` and `runtime` shards (post-PR4b uniform five-shard job) on `windows-11-arm` with the PR6a-recipe engine built by the pin-verifying `dolt-windows-arm64` job and imported through `bundle:prepare --archive --offline` (design D8) -> both shards green uninstrumented (design Open Question 7, lead ruling 1), receipts record host triple `aarch64-pc-windows-msvc` and name the runs as arm64 behavioral evidence never counted toward the 90% gate, run id recorded here
- [ ] 1.2 @e2e (agent) `//apps/kuru-tui:test:embedded-runtime` on `windows-11-arm` with a cold offline engine cache -> install, first conversation, resume and update pass without any download

## 2. Owned process cleanup on Windows on Arm [critical]

- [ ] 2.1 @runtime (agent) `ci.yml native-platform` arm64 leg -> `kuru-platform` tests pass uninstrumented (lead ruling 1) and are recorded as separately named arm64 behavioral evidence outside the 90% gate, which the instrumented x64 and Unix runs keep enforcing; the arm64 coverage upload is held until a pinned toolchain carries the rust-lang/rust#150123 fix
- [ ] 2.2 @integration (agent) `connectors-core-platform` shard on `windows-11-arm` -> supervisor reap-before-release and owned-process tests pass natively

## 3. Native terminal behavior on Windows on Arm [critical]

- [ ] 3.1 @e2e (agent) `application` shard on `windows-11-arm` -> real ConPTY session tests pass with completed-frame synchronization

## 4. Installation on Windows on Arm [critical]

- [ ] 4.1 @e2e (agent) PR5's per-OS install/update job on `windows-11-arm`: `mise run install`, `bundle:verify-native-build`, `verify:windows-imports` -> source install completes, missing and corrupt-mirror builds fail at the memory build-script boundary, the shipping `kuru.exe` and embedded `dolt.exe` are PE ARM64 with OS-only imports
- [ ] 4.2 @integration (agent) bootstrap tests in the `delivery-archive` shard on both Windows runners -> the bootstrap selects the runner's native target while `PROCESSOR_ARCHITECTURE=AMD64` is injected into a native shell (proving the environment is ignored), rejects a mismatched explicit `-Target` with a message naming the native target, and requires the per-target PE machine; if step zero finds an x64 PowerShell host, a case launched from that emulated process also selects `aarch64-pc-windows-msvc`, otherwise the absence is recorded here (phase 2 bridge audit record, task 2.2: one added P/Invoke, `IsWow64Process2` from kernel32 under https://learn.microsoft.com/windows/win32/api/wow64apiset/nf-wow64apiset-iswow64process2, called on the stock .NET `Process.GetCurrentProcess().Handle`, failing closed with `IOException` wrapping `Win32Exception`, with no struct layout change and C# 5 source for stock PowerShell 5.1 `Add-Type`; the injection, mismatch, PE machine and missing-target-archive cases exist in `bootstrap_windows.rs` and are unrun on this macOS host, where no Windows compile is possible; first proven on x64 by the `delivery-archive` shard on the task 2.12 draft PR; open for phase 3: `retained_v041_v042_powershell_reader_accepts_the_new_three_member_core` runs the frozen x64-only v0.4.2 reader, which rejects the ARM64 fixture executable, and needs the named disposition in tasks 3.4 and 3.7 before this row can pass on `windows-11-arm`) (x64 PowerShell launch path, task 3.7, observed 2026-09-26 in step-zero probe run 36289581514: none exists on `windows-11-arm`. System32 `powershell.exe` is 0xAA64, SysWOW64 `powershell.exe` is 0x014C (x86), there is no SysArm32 copy and the only `pwsh.exe` is 0xAA64, so no emulated-host bootstrap case is added. This row's arm64 evidence is therefore the native-process cases (environment injection, `-Target` mismatch, PE machine), which the spec scenarios require; they do not claim an x64-emulated PowerShell launch; retained-reader disposition, 2026-09-27: `retained_v041_v042_powershell_reader_accepts_the_new_three_member_core` keeps its x64 acceptance case, and on a non-x64 target asserts that the frozen reader refuses with "Use native 64-bit Windows PowerShell 5.1 on Windows x64." and leaves the installation unchanged, because that reader rejects any `PROCESSOR_ARCHITECTURE` other than `AMD64` and no x64 PowerShell host exists on `windows-11-arm`; unrun until the Windows on Arm partitions run)
- [x] 4.3 @unit (agent) `targets.rs` test after the catalog entry -> `for_platform("windows","aarch64")` yields `kuru.exe`/`zip`; `x86_64-pc-windows-gnu` still rejected (observed 2026-09-26 on macOS arm64 at `8f4536b7`: `every_native_platform_has_one_consistent_release_name` passes in `//packages/kuru-delivery:test`, exit 0, 251 passed, 0 failed, 1 ignored; `for_platform("windows","aarch64")` yields `aarch64-pc-windows-msvc` with `kuru.exe` and `zip`, and `macos`/`x86_64`, `x86_64-apple-darwin`, `x86_64-pc-windows-gnu` and `aarch64-pc-windows-gnullvm` stay rejected)
- [ ] 4.4 @integration (agent) bootstrap recovery case on both Windows runners -> an interrupted update whose receipt names an x64 helper (`x86_64-pc-windows-msvc-<sha>.exe`, PE machine `0x8664`) is recovered, and a helper whose name and PE machine disagree is rejected (phase 2: the x64-named helper receipt recovers on the x64 runner in the extended crash-gap case, and the disagreement case exercises the name/PE-machine derivation; both unrun here and proven by the `delivery-archive` shard on the draft PR; on `windows-11-arm` the native fixture records an arm64 helper, so the x64-named recovery there needs an x64 `kuru.exe` launched under emulation, tied to the step-zero x64 launch probe (task 3.2) and task 3.7; without that path this row is recorded as limited to the derivation plus the x64-runner case) (task 3.7, observed 2026-09-26 in step-zero probe run 36289581514: there is no x64 PowerShell host on `windows-11-arm`, so this row is limited to the name/PE-machine derivation and the disagreement rejection, which run on both runners, plus the x64-named helper recovery on the `windows-latest` runner. No x64-emulated recovery case is added on `windows-11-arm`)

## 5. Update on Windows on Arm [critical]

- [ ] 5.1 @e2e (agent) `verify-staged` leg `{windows-11-arm, aarch64-pc-windows-msvc}` (mise-route, mirroring `windows-latest`) on the first maintainer-authorized Release run after merge (release-time gate; recorded as an explicit post-merge deferral at archive, task 3.8) -> staged ZIP installs through loopback mise, offline demo persists and resumes, engine and licenses match the manifest, and the log carries either a predecessor-updater success or `no predecessor for aarch64-pc-windows-msvc: inspected ...`
- [ ] 5.2 @unit (agent) `published.rs` selection tests with an in-memory manifest provider -> `predecessor_is_the_greatest_older_release_carrying_the_target`, `predecessor_skips_releases_without_the_target_when_a_later_one_exists`, `no_predecessor_when_no_stable_release_carries_the_target`, `no_predecessor_requires_the_manifest_to_agree`, `no_predecessor_follows_every_listing_page` pass on every host without network access; fetched `SHA256SUMS` are authenticated against their listed digests (observed on macOS arm64 without network access in `//packages/kuru-delivery:test` at pre-rebase `79f972cb` and after the rebase (group 6): all five tests pass, with `no_predecessor_prints_the_exact_evidence_line`, `listing_pages_follow_only_the_exact_next_page` and `token_is_sent_only_on_api_metadata_requests`; unrun: x64 Windows, proven by the `delivery-archive` shard on the draft PR)
- [ ] 5.3 @integration (agent) `test:previous-release-update` in PR5's per-OS install/update job in ordinary CI on `windows-latest`, `macos-latest` and `ubuntu-latest` with `GITHUB_TOKEN` from CI -> the actual previous release's updater installs the candidate; the `Release` branch of the new enum behaves as before; on the `windows-11-arm` leg, until a release carries `aarch64-pc-windows-msvc`, the log carries `no predecessor for aarch64-pc-windows-msvc: inspected ...` (phase 2, observed 2026-09-26 on macOS arm64 against real GitHub, anonymously because `GITHUB_TOKEN` is optional: with `KURU_UPDATE_CANDIDATE_BINARY` set to a release-profile `kuru` built from the rebased branch, the rewritten resolver's `Release` branch selected v0.9.0 for `aarch64-apple-darwin` from the first listing page and authenticated its `SHA256SUMS`, and the v0.9.0 updater installed candidate v0.9.1, 1 passed (`scratchpad/pr6b/stage2/fixes/previous-release-update.log`); unrun: `windows-latest` and `ubuntu-latest`, and the CI `macos-latest` run with CI's token, proven by #110's per-OS install/update job on the task 2.12 draft PR; the `windows-11-arm` `None` line is phase 3)
- [ ] 5.4 @e2e (agent) per-OS install/update job `test:embedded-runtime` on `windows-11-arm` -> `packaged_install_and_update_preserve_complete_offline_memory` passes natively; this is the pre-merge update evidence for the support claim

## 6. x64 stays green through every generalization [critical]

- [ ] 6.1 @regression (agent) full x64 Windows CI (`windows-2025` before PR5, `windows-latest` after; native-platform, five coverage shards, collect, install/update) on every phase-2 commit -> green with no catalog or workflow change; fixtures derive `AMD64` from the native machine (in progress, observed 2026-09-26 on draft PR #115 `https://github.com/replygirl/kuru/pull/115` (`feat/windows-arm64` -> `main`), run `36284936281`: `gh pr checks 115` shows 30 checks total, 14 `pass`, 16 `pending`, 0 `fail`; of the six `windows-latest` jobs, `Native platform primitives (Windows)` is `pass` (3m58s) and the other five (`application`, `connectors-core-platform`, `delivery-archive`, `memory` and `runtime` coverage shards, plus `Installation, offline runtime and update`) are still `pending`/`in_progress` as of this check; no check has failed or been retried; raw `gh pr checks` and `gh run view --json status,conclusion,jobs` output saved to `scratchpad/pr6b/stage2/pr115-checks.txt` and `run-36284936281.json`; not yet green, so this task stays open until every `windows-latest` job in the run reaches `pass`)
- [ ] 6.2 @unit (agent) `release_workflow.rs` gate cases, in the phase-3 workflow commit (task 3.4) because the test reads the live `native-tests.yml` gate script as PR5 rewrites it around shard, collect and install -> `windows-latest` accept/reject cases unchanged, `windows-11-arm` accept and each reject case added, unknown Windows label rejected (shape note, 2026-09-27: PR-C's `native-gate` never reads the caller's label and its test asserts so, so the Windows on Arm accept case, the rejects and the unknown-label rejects are in the `coverage::PARTITIONS` table test, the fail-closed allowlist that every partition and merge enforces; `native_workflow_gate_rejects_incomplete_results` is unchanged and still covers every OS)
- [ ] 6.3 @integration (agent) `//packages/kuru-delivery:test`, `//packages/kuru-memory:test`, `//apps/kuru-tui:test` on macOS arm64 and x64 Windows after phase 2 -> all pass; `cargo check --target aarch64-pc-windows-msvc` for `kuru-platform` passes via the new cross-check task (partial, observed 2026-09-26 on macOS arm64 at `32e5b8f1`: `//packages/kuru-platform:typecheck:windows-arm64` exit 0, the `cargo check --target aarch64-pc-windows-msvc` clause; `//packages/kuru-delivery:test` 202 passed, 0 failed at `8f1fc302`; superseded by the group 6 results below; every x64 Windows run is pending the draft PR; draft PR #115 run `36284936281` observed 2026-09-26: five `windows-latest` coverage/install jobs still `pending`, `Native platform primitives (Windows)` `pass` — see 6.1 for the full check breakdown) (local results, observed 2026-09-26 on macOS arm64 at `c5c4cdda` after the rebase onto `ec759136` and the review follow-ups, every mise call with `MISE_CEILING_PATHS=/Users/rg/repos/rg/kuru/tmp/worktrees` and `MISE_NO_HOOKS=1`: `format:check`, `lint`, `typecheck`, `lint:tooling`, `docs:check` and `cospec:managed:check` exit 0; `//packages/kuru-delivery:test` exit 0, 203 passed, 0 failed, 1 ignored (the live previous-release test, run separately for 5.3); `//apps/kuru-tui:test` exit 0, 245 passed, 0 failed, 4 ignored; `//packages/kuru-memory:test` exit 0, 302 passed, 0 failed (502 s); logs in `scratchpad/pr6b/stage2/checks-round2/`, and the same set was rc=0 at pre-rebase `79f972cb` in `checks-round1/`; one local-only observation: an unlocked mise run (the local 2026.9.13 and, in an isolated copy, the pinned 2026.9.4) rewrites `mise.lock` by dropping `specifiers = ["0.7.1"]` from the cospec `asset_pattern` element, while the same run under CI's `MISE_LOCKED=1` leaves it byte-identical (`scratchpad/pr6b/stage2/autolock/`), so the lock was restored before committing and CI's unchanged-lock step is not affected; unrun, each with its proving job on the task 2.12 draft PR: every `cfg(windows)` test and the x64 Windows compile (`bootstrap_windows.rs`, `windows_update.rs`, `windows_archive.rs`, `windows_cli.rs` and its fixture, `windows_mise.rs` with `mise_acceptance.rs`, the Windows paths of `embedded_runtime.rs`, `published_windows` on Windows and `kuru-platform` `windows_commands.rs`), because `aws-lc-sys` cannot build for `x86_64-pc-windows-msvc` on macOS, proven by the `delivery-archive`, `application` and `connectors-core-platform` Windows coverage shards and ci `native-platform`; `verify-windows-imports.ps1`, `install-source.ps1` and the x64 static-CRT release build, proven by #110's per-OS install/update job on `windows-latest`; `test:previous-release-update` with CI's `GITHUB_TOKEN` on `ubuntu-latest`, `macos-latest` and `windows-latest`, proven by the same job; the lock diff check, proven by that job's `Require unchanged dependency locks` step) (task 3.4 part (b), observed 2026-09-27 on macOS arm64 after the local rebase onto `98b6b81a`, not x64: `format:check`, `lint`, `typecheck` and `lint:tooling` (actionlint, including the nested `bundle-build.yml` call from `native-tests.yml`) exit 0; `//packages/kuru-delivery:test` exit 0, 311 passed, 0 failed, 1 ignored, including `tables_cover_the_workspace_and_hosted_labels`, `windows_on_arm_partitions_are_uninstrumented_behavioral_evidence_with_an_imported_engine`, `native_workflow_partitions_every_os_and_keeps_the_aggregate_fail_closed`, `native_workflow_installs_and_accepts_the_previous_release_update_on_every_os` and `native_workflow_gate_rejects_incomplete_results`; `//apps/kuru-tui:test` exit 0, 245 passed, 0 failed, 4 ignored; logs `partb-*.log` in `scratchpad/pr6b/stage3/`; the x64 Windows run is pending CI)
- [x] 6.4 @integration (agent) staged mise fixture asset ids after the #106 rebase -> ids derive from `CATALOG` position, `SHA256SUMS` has an id past the catalog, all ids are unique, and the `api_fallback` scenario passes on x64 Windows (observed, inherited from #106 at `8225613d`: the ids derive from catalog position with `SHA256SUMS` at `CATALOG.len() + 1`, unique by construction with no separate assertion; `api_fallback` runs inside `acceptance::run` via `apps/kuru-tui/tests/windows_mise.rs` in the `application` shard, "Windows coverage (application)" passed on `windows-2025` in #106's CI run 36270988948, job 108484772452)

## 7. Post-publication verification per target

- [ ] 7.1 @runtime (agent) `verify-published-windows` arm64 leg on the first arm64 release (post-publication; recorded as an explicit post-merge deferral at archive, task 3.8) -> receipt `published-windows-aarch64-pc-windows-msvc-receipt.json` records `target`, PE machine ARM64 and confirmed cleanup; x64 receipt name unchanged apart from the target segment (wiring landed 2026-09-26 at `8f4536b7`: a two-target matrix with `KURU_PUBLISHED_TARGET: ${{ matrix.target }}`, receipt `published-windows-${{ matrix.target }}-receipt.json` and artifact `published-windows-${{ matrix.target }}-<version>-<attempt>`, asserted by `release_workflow.rs`; the run itself is post-publication, task 3.8)
- [ ] 7.2 @unit (agent) receipt schema tests -> a receipt without `target` or with a target differing from the installed PE machine is rejected (observed on macOS arm64 at `685af893`: `receipt_schema_requires_target_matching_its_pe_machine` rejects a receipt without `target`, one whose `pe_machine` differs from its target and a schema 1 receipt; `installed_image_pe_machine_must_match_the_target` rejects an installed image whose PE machine differs from the target; the x64 Windows run is pending)
- [ ] 7.3 @unit (agent) built-asset notice checks after PR6a's v2 schema -> the staged and published verifiers reject a built asset missing, or with a digest-mismatched, ICU, LLVM runtime or mingw-w64 notice, accept an upstream asset without notices, and the arm64 published receipt records the verified notices (partial, observed 2026-09-26 on macOS arm64 at `e383440d` in `//packages/kuru-delivery:test`, exit 0, 251 passed, 0 failed, 1 ignored: `built_engine_missing_notice_is_rejected`, `built_engine_digest_mismatched_notice_is_rejected` (a same-length altered notice and a wrong pinned size), `upstream_engine_without_notices_is_accepted` (the committed x64 entry, with `notices` absent and empty), `built_engine_notice_is_verified_and_recorded` and `engine_notice_declarations_fail_closed` (an upstream asset with notices, a built asset with none, missing or unknown `provenance`, an `"unpinned"` digest, `null` bytes, an unknown notice field, path-like, reserved and duplicate names) pass against real extracted-engine directories; both verifiers call the same `published_windows::verify_engine_notices`, and `public_selector_and_receipt_do_not_contain_endpoint_or_raw_output_fields` covers a receipt carrying `engine.notices`; logs in `scratchpad/pr6b/stage3/`; unrun: the Windows-only staged call, proven by the `application` shard on the draft PR, and the arm64 published receipt itself, a release-time observation under 7.1)

## 8. Tooling and lockfiles on `windows-11-arm`

- [x] 8.1 @manual (agent) step-zero probe job output -> stock PowerShell 5.1 native machine is ARM64 under both shells; `VC.Tools.ARM64` and `dumpbin.exe` paths listed; `rustc --print host-tuple` is `aarch64-pc-windows-msvc`; `cospec --version` and `cog --version` results, whether an x64 PowerShell host launches, the runner image version and Visual Studio edition/version (README and `vswhere`) recorded because the label migrates to Visual Studio 2026 during 2026-09-21 to 2026-09-30 (actions/runner-images#14602), and the instrumented `cargo llvm-cov` smoke result recorded verbatim, including `llvm-profdata merge -sparse` stderr so rust-lang/rust#150123 is confirmed or ruled out on Rust 1.98.1 (recorded as the pinned toolchain's state; per design Open Question 7, lead ruling 1, the arm64 suites stay uninstrumented until a pinned toolchain carries the fix); also clang-cl presence (`vswhere -requires Microsoft.VisualStudio.Component.VC.Llvm.Clang`, the located `clang-cl.exe`) and an `aws-lc-sys` build smoke for `aarch64-pc-windows-msvc`, whose build script requires clang-cl for Windows Arm64 (observed on the macOS cross-check: "Windows ARM64 requires clang-cl"; a missing component blocks every arm64 build job)

  Step-zero probe (task 3.2), observed 2026-09-26 on PR #115 via the temporary `.github/workflows/probe-windows-arm64.yml` (to be removed before merge). Evidence run: https://github.com/replygirl/kuru/actions/runs/36289581514/job/108536962446 (commit ab92798f on the PR merge ref with main 9def614b); earlier runs 36289122306 and 36289378393 (commits f9d82b74, 7cef5d50) recorded the same machine/shell/VS/cospec/cog results but lost the cargo smokes to the mise-shim finding below. Full log: `scratchpad/pr6b/stage3/probe/probe.log` (run 1 and 2 logs beside it).

  - Label resolves: yes; `windows-11-arm` picked the job up within minutes; ImageOS `win11-vs2026-arm64`, ImageVersion `20260920.164.1`, Windows 11 Enterprise 10.0.26200, OSArchitecture "ARM 64-bit Processor". ImageOS/ImageVersion are recorded in lieu of the image README; the version identifies the runner-images release.
  - Native machine: stock Windows PowerShell 5.1.26100.9457 (Desktop, System32, Arm64 process) and pwsh 7.6.6 (Core, Arm64 process) both report `IsWow64Process2 processMachine=0x0000 nativeMachine=0xAA64` through `Add-Type` (Add-Type=ok in both); `PROCESSOR_ARCHITECTURE=ARM64`, `PROCESSOR_ARCHITEW6432` empty.
  - x64 PowerShell host: none. System32 powershell.exe is 0xAA64, SysWOW64 powershell.exe is 0x014C (x86), no SysArm32 copy, the only pwsh.exe (Program Files\PowerShell\7) is 0xAA64. Task 3.7 therefore takes the no-launch-path branch.
  - Visual Studio: Visual Studio Enterprise 2026, installationVersion 18.10.12210.168; `-requires Microsoft.VisualStudio.Component.VC.Tools.ARM64` and `-requires Microsoft.VisualStudio.Component.VC.Llvm.Clang` both resolve to `C:\Program Files\Microsoft Visual Studio\18\Enterprise`. Native ARM64 dumpbin: `...\VC\Tools\MSVC\14.51.36231\bin\Hostarm64\arm64\dumpbin.exe` (0xAA64; also 14.44.35207). vswhere.exe itself is x86 (0x014C) and ran under emulation.
  - clang-cl: present. VS component `...\VC\Tools\Llvm\ARM64\bin\clang-cl.exe` (0xAA64) and `...\Llvm\x64\bin\clang-cl.exe`; PATH resolves `C:\Program Files\LLVM\bin\clang-cl.exe` (image LLVM). cmake on PATH; nasm and dumpbin not on PATH.
  - Host tuple: image rustc and pinned `rustup run 1.98.1 rustc --print host-tuple` both `aarch64-pc-windows-msvc`; installed target `aarch64-pc-windows-msvc` (image default also has `aarch64-pc-windows-gnullvm`). mise 2026.9.4 is a native windows-arm64 build (0xAA64).
  - cospec/cog under emulation: yes. `MISE_LOCKED=1` installed `cospec-0.8.2-windows-x64.zip` (cospec.exe 0x8664), `mise run cospec -- --version` printed `0.8.2`, exit 0. `setup:test-tools` installed `cocogitto-7.0.0-x86_64-pc-windows-msvc.tar.gz` (cog.exe 0x8664) and native `communique-aarch64-pc-windows-msvc.zip`; `cog --version` printed `cog 7.0.0`, exit 0. No exclusion is needed (lead ruling 5; task 3.3 takes the "cospec ran" branch).
  - aws-lc-sys: built. `cargo +1.98.1 check -p kuru-delivery --features tooling --locked` exit 0 in 1m 28s; the build script set `CC_aarch64_pc_windows_msvc: clang-cl` and found clang-cl on PATH (its `stdalign_check.c` feature probe fails under -WX, which is a probe, not a build failure).
  - Instrumented llvm-cov smoke: rust-lang/rust#150123 CONFIRMED on Rust 1.98.1 (LLVM 22.1.8-rust-1.98.1-stable). The instrumented `kuru-archive` test passed, but `llvm-profdata merge -sparse` rejected all 3 profraw files with "malformed instrumentation profile data: symbol name is empty" and "error: no profile can be merged" (exit 1); `cargo llvm-cov report` failed identically. This matches the issue's reported signature verbatim ("malformed instrumentation profile data: symbol name is empty" / "error: no profile can be merged" from `llvm-profdata merge -sparse` on `aarch64-pc-windows-msvc`; issue open as of 2026-09-26). The failing tools were native: the profraw files came from the ARM64 instrumented test binary and the `llvm-profdata.exe` under `1.98.1-aarch64-pc-windows-msvc` is the toolchain's own; the x64 `cargo-llvm-cov` under emulation only orchestrates. The instrumented-arm64 hold (lead ruling 1) stays in force.
  - New finding (lockfile): with main's #114, root `aqua:taiki-e/cargo-llvm-cov@0.9.1` has no `platforms.windows-arm64` lock entry, so `MISE_LOCKED=1 mise install` fails ("No lockfile URL found ... on platform windows-arm64 (--locked mode)") and every `mise x` or mise shim (cargo, rustc, rustup) first attempts that install and fails before running its command. Unlocked, aqua selects `cargo-llvm-cov-x86_64-pc-windows-msvc.tar.gz` (0x8664) although upstream publishes `cargo-llvm-cov-aarch64-pc-windows-msvc.{tar.gz,zip}`. Task 3.4's arm64 legs need a windows-arm64 lock entry for it (or a platform asset_pattern selecting the native asset) after rebasing onto main.

  Delimited probe output (run 36289581514, verbatim; aws-lc-sys section filtered to compiler lines, mise progress-bar frames dropped, ANSI stripped):

  ```text
  ===== PROBE: runner image =====
  ImageOS=win11-vs2026-arm64 ImageVersion=20260920.164.1 RUNNER_ARCH=ARM64
  OS=Microsoft Windows NT 10.0.26200.0
  Caption        : Microsoft Windows 11 Enterprise
  Version        : 10.0.26200
  BuildNumber    : 26200
  OSArchitecture : ARM 64-bit Processor
  ===== END: runner image =====
  ===== PROBE: native machine, stock Windows PowerShell =====
  PSEdition=Desktop PSVersion=5.1.26100.9457
  Name                      Value                  
  ----                      -----                  
  PSVersion                 5.1.26100.9457         
  PSEdition                 Desktop                
  PSCompatibleVersions      {1.0, 2.0, 3.0, 4.0...}
  BuildVersion              10.0.26100.9457        
  CLRVersion                4.0.30319.42000        
  WSManStackVersion         3.0                    
  PSRemotingProtocolVersion 2.3                    
  SerializationVersion      1.1.0.1
  ProcessPath=C:\Windows\System32\WindowsPowerShell\v1.0\powershell.EXE
  Is64BitProcess=True
  RuntimeInformation.ProcessArchitecture=Arm64 OSArchitecture=Arm64
  PROCESSOR_ARCHITECTURE=ARM64 PROCESSOR_ARCHITEW6432=
  Add-Type=ok
  IsWow64Process2 processMachine=0x0000 nativeMachine=0xAA64
  ===== END: native machine, stock Windows PowerShell =====
  ===== PROBE: native machine, pwsh =====
  PSEdition=Core PSVersion=7.6.6
  Name                      Value
  ----                      -----
  PSVersion                 7.6.6
  PSEdition                 Core
  GitCommitId               7.6.6
  OS                        Microsoft Windows 10.0.26200
  Platform                  Win32NT
  PSCompatibleVersions      {1.0, 2.0, 3.0, 4.0…}
  PSRemotingProtocolVersion 2.4
  SerializationVersion      1.1.0.1
  WSManStackVersion         3.0
  ProcessPath=C:\Program Files\PowerShell\7\pwsh.EXE
  Is64BitProcess=True
  RuntimeInformation.ProcessArchitecture=Arm64 OSArchitecture=Arm64
  PROCESSOR_ARCHITECTURE=ARM64 PROCESSOR_ARCHITEW6432=
  Add-Type=ok
  IsWow64Process2 processMachine=0x0000 nativeMachine=0xAA64
  ===== END: native machine, pwsh =====
  ===== PROBE: x64 PowerShell host =====
  candidate: C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe machine=0xAA64
  candidate: C:\Windows\SysWOW64\WindowsPowerShell\v1.0\powershell.exe machine=0x014C
  absent: C:\Windows\SysArm32\WindowsPowerShell\v1.0\powershell.exe
  candidate: C:\Program Files\PowerShell\7\pwsh.exe machine=0xAA64
  RESULT: no x64 PowerShell host found
  ===== END: x64 PowerShell host =====
  ===== PROBE: Visual Studio =====
  vswhere=C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe exists=True machine=0x014C
  displayName=Visual Studio Enterprise 2026 installationVersion=18.10.12210.168 productId=Microsoft.VisualStudio.Product.Enterprise path=C:\Program Files\Microsoft Visual Studio\18\Enterprise
  requires VC.Tools.ARM64 -> [C:\Program Files\Microsoft Visual Studio\18\Enterprise]
  requires VC.Llvm.Clang -> [C:\Program Files\Microsoft Visual Studio\18\Enterprise]
  --- dumpbin.exe:
  C:\Program Files\Microsoft Visual Studio\18\Enterprise\VC\Tools\MSVC\14.29.30133\bin\HostX64\arm\dumpbin.exe machine=0x8664
  C:\Program Files\Microsoft Visual Studio\18\Enterprise\VC\Tools\MSVC\14.29.30133\bin\HostX64\arm64\dumpbin.exe machine=0x8664
  C:\Program Files\Microsoft Visual Studio\18\Enterprise\VC\Tools\MSVC\14.29.30133\bin\HostX64\x64\dumpbin.exe machine=0x8664
  C:\Program Files\Microsoft Visual Studio\18\Enterprise\VC\Tools\MSVC\14.29.30133\bin\HostX64\x86\dumpbin.exe machine=0x8664
  C:\Program Files\Microsoft Visual Studio\18\Enterprise\VC\Tools\MSVC\14.29.30133\bin\HostX86\arm\dumpbin.exe machine=0x014C
  C:\Program Files\Microsoft Visual Studio\18\Enterprise\VC\Tools\MSVC\14.29.30133\bin\HostX86\arm64\dumpbin.exe machine=0x014C
  C:\Program Files\Microsoft Visual Studio\18\Enterprise\VC\Tools\MSVC\14.29.30133\bin\HostX86\x64\dumpbin.exe machine=0x014C
  C:\Program Files\Microsoft Visual Studio\18\Enterprise\VC\Tools\MSVC\14.29.30133\bin\HostX86\x86\dumpbin.exe machine=0x014C
  C:\Program Files\Microsoft Visual Studio\18\Enterprise\VC\Tools\MSVC\14.44.35207\bin\Hostarm64\arm64\dumpbin.exe machine=0xAA64
  C:\Program Files\Microsoft Visual Studio\18\Enterprise\VC\Tools\MSVC\14.44.35207\bin\Hostarm64\x64\dumpbin.exe machine=0xAA64
  C:\Program Files\Microsoft Visual Studio\18\Enterprise\VC\Tools\MSVC\14.44.35207\bin\Hostx64\arm64\dumpbin.exe machine=0x8664
  C:\Program Files\Microsoft Visual Studio\18\Enterprise\VC\Tools\MSVC\14.44.35207\bin\Hostx64\x64\dumpbin.exe machine=0x8664
  C:\Program Files\Microsoft Visual Studio\18\Enterprise\VC\Tools\MSVC\14.44.35207\bin\Hostx86\arm64\dumpbin.exe machine=0x014C
  C:\Program Files\Microsoft Visual Studio\18\Enterprise\VC\Tools\MSVC\14.44.35207\bin\Hostx86\x86\dumpbin.exe machine=0x014C
  C:\Program Files\Microsoft Visual Studio\18\Enterprise\VC\Tools\MSVC\14.51.36231\bin\Hostarm64\arm64\dumpbin.exe machine=0xAA64
  C:\Program Files\Microsoft Visual Studio\18\Enterprise\VC\Tools\MSVC\14.51.36231\bin\Hostarm64\x64\dumpbin.exe machine=0xAA64
  C:\Program Files\Microsoft Visual Studio\18\Enterprise\VC\Tools\MSVC\14.51.36231\bin\Hostarm64\x86\dumpbin.exe machine=0xAA64
  C:\Program Files\Microsoft Visual Studio\18\Enterprise\VC\Tools\MSVC\14.51.36231\bin\Hostx64\arm64\dumpbin.exe machine=0x8664
  C:\Program Files\Microsoft Visual Studio\18\Enterprise\VC\Tools\MSVC\14.51.36231\bin\Hostx64\x64\dumpbin.exe machine=0x8664
  C:\Program Files\Microsoft Visual Studio\18\Enterprise\VC\Tools\MSVC\14.51.36231\bin\Hostx64\x86\dumpbin.exe machine=0x8664
  C:\Program Files\Microsoft Visual Studio\18\Enterprise\VC\Tools\MSVC\14.51.36231\bin\Hostx86\arm64\dumpbin.exe machine=0x014C
  C:\Program Files\Microsoft Visual Studio\18\Enterprise\VC\Tools\MSVC\14.51.36231\bin\Hostx86\x64\dumpbin.exe machine=0x014C
  C:\Program Files\Microsoft Visual Studio\18\Enterprise\VC\Tools\MSVC\14.51.36231\bin\Hostx86\x86\dumpbin.exe machine=0x014C
  --- clang-cl.exe:
  C:\Program Files\Microsoft Visual Studio\18\Enterprise\VC\Tools\Llvm\ARM64\bin\clang-cl.exe machine=0xAA64
  C:\Program Files\Microsoft Visual Studio\18\Enterprise\VC\Tools\Llvm\x64\bin\clang-cl.exe machine=0x8664
  --- PATH lookup:
  clang-cl.exe -> C:\Program Files\LLVM\bin\clang-cl.exe
  clang.exe -> C:\Program Files\LLVM\bin\clang.exe
  cmake.exe -> C:\Program Files\CMake\bin\cmake.exe
  ===== END: Visual Studio =====
  ===== PROBE: image toolchain (not pinned) =====
  rustc 1.98.1 (48a229cea 2026-09-01)
  binary: rustc
  commit-hash: 48a229ceaefd4985c50990b14116b6d856af0985
  commit-date: 2026-09-01
  host: aarch64-pc-windows-msvc
  release: 1.98.1
  LLVM version: 22.1.8
  Default host: aarch64-pc-windows-msvc
  rustup home:  C:\Users\runneradmin\.rustup

  installed toolchains
  --------------------
  stable-aarch64-pc-windows-msvc (active, default)

  active toolchain
  ----------------
  name: stable-aarch64-pc-windows-msvc
  active because: it's the default toolchain
  installed targets:
    aarch64-pc-windows-gnullvm
    aarch64-pc-windows-msvc
  ===== END: image toolchain (not pinned) =====
  ===== PROBE: pinned toolchain =====
  direct cargo=C:\Users\runneradmin\.cargo\bin\cargo.exe rustup=C:\Users\runneradmin\.cargo\bin\rustup.exe
  2026.9.4 windows-arm64 (2026-09-09)
  mise machine=0xAA64
  PATH cargo=C:\Users\runneradmin\.cargo\bin\cargo.exe rustc=C:\Users\runneradmin\.cargo\bin\rustc.exe
  rustc 1.98.1 (48a229cea 2026-09-01)
  binary: rustc
  commit-hash: 48a229ceaefd4985c50990b14116b6d856af0985
  commit-date: 2026-09-01
  host: aarch64-pc-windows-msvc
  release: 1.98.1
  LLVM version: 22.1.8
  aarch64-pc-windows-msvc
  aarch64-pc-windows-msvc
  ===== END: pinned toolchain =====
  ===== PROBE: mise install (MISE_LOCKED=1) =====
  MISE_LOCKED=1
  mise by @jdx – installing 8 tools
  mise ⇢ rust@1.98.1                        62ms · already installed
  mise ✗ aqua:taiki-e/cargo-llvm-cov@0.9.1  2ms · failed: No lockfile URL found for aqua:taiki-e/cargo-llvm-cov@0.9.1 on platform windows-arm64 (--locked mode)
  mise ✓ aqua:tamasfe/taplo@0.10.0          815ms  taplo-windows-aarch64.zip
  mise ✓ aqua:jdx/hk@2.2.0                  967ms  hk-aarch64-pc-windows-msvc.zip
  mise ✓ aqua:koalaman/shellcheck@0.11.0    1.0s  shellcheck-v0.11.0.zip
  mise ✓ aqua:rhysd/actionlint@1.7.12       1.2s  actionlint_1.7.12_windows_arm64.zip
  mise ✓ mr-boxington@1.18.0                1.9s  mbx.powershell
  mise ✓ github:aligned-team/cospec@0.8.2   2.7s  cospec-0.8.2-windows-x64.zip
  mise ERROR Failed to install aqua:taiki-e/cargo-llvm-cov@0.9.1: No lockfile URL found for aqua:taiki-e/cargo-llvm-cov@0.9.1 on platform windows-arm64 (--locked mode)
  hint: Run `mise lock` to generate lockfile URLs, or disable locked mode
  mise ERROR Version: 2026.9.4 windows-arm64 (2026-09-09)
  mise ERROR Run with --verbose or MISE_VERBOSE=1 for more information
  mise install exit=1
  aqua:jdx/hk                  2.2.0             C:\a\kuru\kuru\mise.toml  2.2.0
  aqua:koalaman/shellcheck     0.11.0            C:\a\kuru\kuru\mise.toml  0.11.0
  aqua:rhysd/actionlint        1.7.12            C:\a\kuru\kuru\mise.toml  1.7.12
  aqua:taiki-e/cargo-llvm-cov  0.9.1 (missing)   C:\a\kuru\kuru\mise.toml  0.9.1
  aqua:tamasfe/taplo           0.10.0            C:\a\kuru\kuru\mise.toml  0.10.0
  github:aligned-team/cospec   0.8.2             C:\a\kuru\kuru\mise.toml  0.8.2
  mr-boxington                 1.18.0            C:\a\kuru\kuru\mise.toml  1.18.0
  rust                         1.98.1 (symlink)  C:\a\kuru\kuru\mise.toml  1.98.1
  ===== END: mise install (MISE_LOCKED=1) =====
  ===== PROBE: cospec and cog =====
  cospec binary=C:\Users\runneradmin\AppData\Local\mise\installs\github-aligned-team-cospec\0.8.2\cospec.exe machine=0x8664
  [//:cospec] $ cospec --version
  0.8.2
  cospec exit=0
  [//packages/kuru-delivery:setup:test-too…] $ mise install aqua:cocogitto/cocogitto@7.0.0 github:jdx/communique@1.4.2
  mise by @jdx – installing 2 tools
  mise ✓ aqua:cocogitto/cocogitto@7.0.0  400ms  cocogitto-7.0.0-x86_64-pc-windows-msvc.tar.gz
  mise ✓ github:jdx/communique@1.4.2     796ms  communique-aarch64-pc-windows-msvc.zip
  mise WARN  aqua:cocogitto/cocogitto, github:jdx/communique installed but not activated — they are not in any config file.
  To install and activate, run:
    mise use aqua:cocogitto/cocogitto
    mise use github:jdx/communique
  setup:test-tools exit=0
  cog binary=C:\Users\runneradmin\AppData\Local\mise\installs\aqua-cocogitto-cocogitto\7.0.0\x86_64-pc-windows-msvc\cog.exe machine=0x8664
  cog 7.0.0
  cog exit=0
  ===== END: cospec and cog =====
  ===== PROBE: aws-lc-sys build smoke =====
  direct cargo=C:\Users\runneradmin\.cargo\bin\cargo.exe rustup=C:\Users\runneradmin\.cargo\bin\rustup.exe
    Downloaded cmake v0.1.58
     Compiling cmake v0.1.58
      Finished `dev` profile [unoptimized + debuginfo] target(s) in 1m 28s
  cargo check exit=0
  --- aws-lc-sys build script lines mentioning clang/cmake/cc-builder/requires/error:
  cargo:warning=Setting CC_aarch64_pc_windows_msvc: clang-cl
  cargo:warning=Environment Variable found 'CC_aarch64_pc_windows_msvc': 'clang-cl'
  cargo:warning=Setting CC_aarch64_pc_windows_msvc: clang-cl
  cargo:warning=C:\Users\RUNNER~1\CARGO~1\registry\src\INDEXC~1.IO-\AWS-LC~1.0\aws-lc\tests\compiler_features_tests\stdalign_check.c(12,14): error: unused parameter 'argc' [-Werror,-Wunused-parameter]
  cargo:warning=C:\Users\RUNNER~1\CARGO~1\registry\src\INDEXC~1.IO-\AWS-LC~1.0\aws-lc\tests\compiler_features_tests\stdalign_check.c(12,27): error: unused parameter 'argv' [-Werror,-Wunused-parameter]
  cargo:warning=2 errors generated.
  cargo:warning=Compilation of 'stdalign_check.c' failed - Err(Error { kind: ToolExecError, message: "command did not execute successfully (status code exit code: 1): \"clang-cl\" \"-nologo\" \"-MD\" \"-Z7\" \"-Brepro\" \"--target=aarch64-pc-windows-msvc\" \"-WX\" \"-W4\" \"-FoC:\\\\a\\\\kuru\\\\kuru\\\\target\\\\debug\\\\build\\\\AWS-LC~4\\\\out\\\\out-stdalign_check\\\\2dcf3a9cab36088c-stdalign_ch
  ===== END: aws-lc-sys build smoke =====
  ===== PROBE: instrumented llvm-cov smoke =====
  direct cargo=C:\Users\runneradmin\.cargo\bin\cargo.exe rustup=C:\Users\runneradmin\.cargo\bin\rustup.exe
  info: downloading component llvm-tools
  mise by @jdx – installing 1 tool
  mise ✓ aqua:taiki-e/cargo-llvm-cov@0.9.1  1.6s  cargo-llvm-cov-x86_64-pc-windows-msvc.tar.gz
  unlocked mise install cargo-llvm-cov exit=0
  cargo-llvm-cov=C:\Users\runneradmin\AppData\Local\mise\installs\aqua-taiki-e-cargo-llvm-cov\0.9.1\cargo-llvm-cov.exe
  cargo-llvm-cov machine=0x8664
  cargo-llvm-cov 0.9.1
  info: cargo-llvm-cov currently setting cfg(coverage); you can opt-out it by passing --no-cfg-coverage
     Compiling crc32fast v1.5.2
     Compiling adler2 v2.0.1
     Compiling simd-adler32 v0.3.10
     Compiling cfg-if v1.0.4
     Compiling hashbrown v0.17.1
     Compiling anyhow v1.0.104
     Compiling miniz_oxide v0.9.1
     Compiling equivalent v1.0.2
     Compiling typed-path v0.12.3
     Compiling memchr v2.8.3
     Compiling indexmap v2.14.2
     Compiling flate2 v1.1.10
     Compiling zip v8.6.0
     Compiling kuru-archive v0.9.0 (C:\a\kuru\kuru\packages\kuru-archive)
      Finished `test` profile [unoptimized + debuginfo] target(s) in 5.54s
       Running unittests src\lib.rs (target\llvm-cov-target\debug\deps\kuru_archive-7c9d3df30f179d1f.exe)

  running 1 test
  test zip::tests::stored_deflated_and_empty_members_decode_without_assuming_writer_layout ... ok

  test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 8 filtered out; finished in 0.00s

  cargo llvm-cov --no-report exit=0
  llvm-profdata=C:\Users\runneradmin\.rustup\toolchains\1.98.1-aarch64-pc-windows-msvc\lib\rustlib\aarch64-pc-windows-msvc\bin\llvm-profdata.exe exists=True
  LLVM (http://llvm.org/):
    LLVM version 22.1.8-rust-1.98.1-stable
    Optimized build.
  profraw files: 3
  llvm-profdata merge -sparse: warning: C:\a\kuru\kuru\target\llvm-cov-target\kuru-3536-15794415139750836131_0.profraw: malformed instrumentation profile data: symbol name is empty
  llvm-profdata merge -sparse: warning: C:\a\kuru\kuru\target\llvm-cov-target\kuru-8316-10589015400864982194_0.profraw: malformed instrumentation profile data: symbol name is empty
  llvm-profdata merge -sparse: warning: C:\a\kuru\kuru\target\llvm-cov-target\kuru-6600-11249919341843589666_0.profraw: malformed instrumentation profile data: symbol name is empty
  llvm-profdata merge -sparse: error: no profile can be merged
  llvm-profdata merge -sparse exit=1
  warning: C:\a\kuru\kuru\target\llvm-cov-target\kuru-3536-15794415139750836131_0.profraw: malformed instrumentation profile data: symbol name is empty
  warning: C:\a\kuru\kuru\target\llvm-cov-target\kuru-8316-10589015400864982194_0.profraw: malformed instrumentation profile data: symbol name is empty
  warning: C:\a\kuru\kuru\target\llvm-cov-target\kuru-6600-11249919341843589666_0.profraw: malformed instrumentation profile data: symbol name is empty
  error: no profile can be merged
  error: failed to merge profile data: process didn't exit successfully: `C:\Users\runneradmin\.rustup\toolchains\1.98.1-aarch64-pc-windows-msvc\lib\rustlib\aarch64-pc-windows-msvc\bin\llvm-profdata.exe merge -sparse -f C:\a\kuru\kuru\target\llvm-cov-target\kuru-profraw-list -o C:\a\kuru\kuru\target\llvm-cov-target\kuru.profdata` (exit code: 1)
  cargo llvm-cov report exit=1
  ===== END: instrumented llvm-cov smoke =====
  ```

- [ ] 8.2 @integration (agent) `mise install` under `MISE_LOCKED=1` on `windows-11-arm` with the `windows-arm64` lock entries -> every job's `install_args` tool installs; the emulated x64 cospec asset is tried first, and only if cospec still does not run is the named `cospec_contract` exclusion present in the shard receipt with its reason and nothing else skipped; if cospec runs, the receipt records that and nothing is excluded (lead ruling 5) (partial, observed 2026-09-26: step-zero probe run 36289581514 installed every root tool under `MISE_LOCKED=1` except `aqua:taiki-e/cargo-llvm-cov`, which had no `windows-arm64` lock entry after #114. The root and `packages/kuru-delivery` locks now carry that entry, generated by `mise lock --platform windows-arm64 aqua:taiki-e/cargo-llvm-cov` and recording the x64 `cargo-llvm-cov-x86_64-pc-windows-msvc.tar.gz` that the aqua registry's `windows_arm_emulation` selects (design D7). The emulated x64 cospec ran in the probe (task 3.3), so no exclusion applies. Unrun: the locked install in the arm64 CI legs and the shard receipt's record, which wait for task 3.4 part (b))
- [ ] 8.3 @regression (agent) root `mise.toml` cospec `platforms.windows-arm64.asset_pattern` -> `mise lock` generates the `windows-arm64` entry naming `cospec-0.7.1-windows-x64.zip`, a second `mise lock` produces no diff, the cocogitto `windows-arm64` entry records the x64 asset aqua selected, and a deliberately non-matching `asset_pattern` never falls back to autodetection: `mise lock` skips the platform silently (exit 0, "Updated 0 platform entries (1 skipped)", no `windows-arm64` block) and only a later `MISE_LOCKED=1 mise install` fails, so the documented refresh confirms the cospec `platforms.windows-arm64` block exists after each lock, and `kuru-delivery repo` in `lint:tooling` fails when that element, its `specifiers`, `checksum` or `url` is missing. Known limitation (design D7): unlocked installs drop the non-host option element (`update_lockfiles`/`merge_tool_entries`), and auto-lock re-adds only the row without `specifiers` or, on a GitHub API 403, nothing. Lockfile-stable local runs therefore use `MISE_LOCKED=1` (https://mise.jdx.dev/dev-tools/mise-lock.html). Repro: unlocked `mise run //apps/kuru-docs:format:check` gives `mise.lock | 14 -` under an anonymous API rate limit (`scratchpad/pr6b/stage3/unlocked-docs/`). Unlocked `mise run format:check` gives the one-line `specifiers` loss (`stage3/v0/c_format_check.diff`, and `stage2/lock-repro/` on 2026.9.13 and 2026.9.4). `mise lock` restores both. With `MISE_LOCKED=1`, `mise ls`, `mise x -- cospec --version`, `mise run format:check` and `mise lock` leave all three lockfiles unchanged (`stage3/locked/*.stat`, all empty, mise 2026.9.13). The repository check rejects both damaged shapes on the real lock and in `repo_validation.rs` (observed 2026-09-26 in isolated copies with the pinned mise 2026.9.4: the correct pattern regenerates the committed block byte for byte (`scratchpad/pr6b/stage2/locktest/pos/pos.log`), a second lock with 2026.9.4 and 2026.9.13 produces no diff (`locktest/lock94.log`), and the non-matching pattern behaves as above (`locktest/neg/neg2.log`); the post-lock check is in `docs/development.md`; the cocogitto entry is `cocogitto-7.0.0-x86_64-pc-windows-msvc.tar.gz`, identical to its `windows-x64` entry; unrun: the CI lock diff check on the draft PR)

## 9. Documentation and support claim

- [ ] 9.1 @manual (human) review `docs/install.md`, `docs/release.md`, `docs/development.md` in the final phase-3 commit -> the Windows on Arm row and any "supported" wording appear only after groups 1 to 4 and row 5.4 are green on the branch (release-time rows 5.1 and 7.1 are post-merge gates that block `deploy-docs` and promotion); `docs:check` passes
- [ ] 9.2 @regression (agent) `cospec validate --strict` and `cospec:managed:check` before archive -> pass
