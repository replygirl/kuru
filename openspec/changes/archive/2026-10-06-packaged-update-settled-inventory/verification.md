# Verification

## 1. Packaged settled installation [critical]

- [x] 1.1 @regression (agent) run the existing packaged offline installation/update roundtrip with the prepared ordinary embedded release input -> owning task 25263 exited 0, exact 1/1 passed in 51.66s (92.85s task); macOS aarch64 current-head release input, direct and updated cold conversations, exact inventory, private lock-only state, recovery returning None and all original checks passed. Official Linux ARM job 112472225608 passed both conversations then failed the obsolete count before correction.
- [x] 1.2 @integration (agent) run the existing application closing-scope structural guard -> owning task 37269 exited 0, exact guard 1/1 passed in 0.11s (96.83s owning task including prepared supervisor and compile); all other targets selected zero cases.
- [~] 1.3 @runtime (agent) execute corrected packaged acceptance on native Linux ARM and Windows -> defer: local macOS verification cannot establish native ARM/Windows execution; Product owns fresh full PR CI.
- [~] 1.4 @regression (agent) run `replaced_linux_running_image_opens_its_own_managed_memory_without_candidate_execution` with cleanup registered at the CLI-resolved Kuru data root -> defer: Linux-only fixture cannot execute on this macOS host; official Ubuntu1 job112473036388 establishes the missing quiescence record before correction, and fresh corrected native CI must establish after behavior.

## 2. Scoped static checks

- [x] 2.1 @integration (agent) run affected app host/Windows lint and all-target typecheck, docs, format and managed checks -> final app host lint 87745, Windows-target lint 48184, all-target/all-feature typecheck 84671 and format 2499 exited 0; owning docs 63541 and managed 97268 exited 0. The Linux-only line is source-reviewed and awaits native Linux CI; no production or configuration changes.

## Evidence

The source review confirms `unix_update::State::open` retains a checked owner-only `.kuru-update/install.lock`, while the existing settled-state test requires only that lock and `recover(parent) == None`. No production lifecycle failure is inferred from the obsolete final fixture count.

The Linux fixture passed its mapped-image body, then the guarded root reported `data/kuru/memory/...` without a quiescence record. `cli::paths` appends `kuru` to XDG_DATA_HOME, while ServiceCleanup enumerates the registered root's `memory` directory. The corrected registration agrees with that existing path contract; it does not establish whether the owner was still alive at the original error.

Before implementation, strict validation and apply task 92e968 each exited 0 and all four returned context files were read. Before the authorized cleanup registration extension, refreshed strict/apply task 61dbbd each exited 0 and all four contexts were reread. Root independently cleared the frozen inventory diff and approved the one-line cleanup registration. Current-head release preparation 93121 exited 0 in 194.01s; its default build emitted existing unused test-helper warnings, without failure.

An initial combined mise check invocation 68336 exited 101 because additional task names were forwarded as Cargo arguments. It did not diagnose a source failure; separate owning invocations passed. Docs and managed results remain applicable after the fixture-only registration line; no duplicate native, combined coverage or paid suite was run. Native ARM/Windows and the Linux-only fixture remain explicitly pending fresh CI rather than counted as local passes.

Both source files stayed frozen during their final selected owning runs. Root's final independent diff review cleared exact platform names and checked lock settlement, plus the one-line actual Kuru cleanup registration with all launch/body assertions untouched. All local handles are terminal; final strict/apply context review and physical archive verification precede the normal hooked commit and Product's remote delivery.
