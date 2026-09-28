# Design

## Context

`profile_env_sha256` digested a `BTreeMap` made from two sources: the eight named process variables in `PROFILE_ENV` (`env:<name>`, `<unset>` when absent) and every variable printed by the pinned cargo-llvm-cov's `llvm-cov show-env --pwsh` (`show-env:<name>`). Before digesting, it replaced only the partition's target path with `${KURU_COVERAGE_TARGET}`. In the cargo-llvm-cov v0.9.1 source (tag object `0dc90567`):

- `src/main.rs:66-75`: `Subcommand::ShowEnv` calls `set_env(cx, writer, IsNextest(true))` unconditionally and then prints `CARGO_LLVM_COV_TARGET_DIR` and `CARGO_LLVM_COV_BUILD_DIR`.
- `src/main.rs:207-246`: when `LLVM_PROFILE_FILE_NAME` is unset, the file name is `{cx.ws.name}-%p` plus, when `is_nextest`, `-%{N}m` with `N = std::thread::available_parallelism().map_or(1, usize::from)`. The TODO at `:234-236` records that N is not clamped to the 1-9 range the rustc documentation states. The result is joined to the target directory. The orchestrator's preset `LLVM_PROFILE_FILE` is not an input and is overridden.
- `src/main.rs:143-148`: `ShowEnvWriter::unset` prints nothing, so an absent key is part of the digest too.

The pinned binary run locally (`mise bin-paths aqua:taiki-e/cargo-llvm-cov@0.9.1`, macOS arm64, `hw.ncpu=14`) prints `LLVM_PROFILE_FILE=<target>/fix-coverage-profile-env-host-token-%p-%14m.profraw`. The checked-in macOS fixture shows `%14m`, and the Linux and Windows fixtures show `%4m`. The CI digests were reconstructed from the tree at `95c79127`, with crate names from `cargo metadata`, the neutral target, the mise install path of `RUSTC_WRAPPER` and the CI variable values. With `N=3` the result is `46e05ee65f11…` (partitions 1, 3 and 4) and with `N=5` it is `0a632ea0fb44…` (partition 2), byte for byte. No other value needed to change.

Host-dependent values in the digested set, and their treatment:

| Key | Source | Depends on | Treatment |
|---|---|---|---|
| `show-env:LLVM_PROFILE_FILE` `%<N>m` | `available_parallelism()` (`main.rs:231-238`) | host CPU count | neutralised (this change) |
| `show-env:LLVM_PROFILE_FILE` directory | `cx.ws.target_dir` | target path | neutralised (existing) |
| `show-env:LLVM_PROFILE_FILE` stem | `cx.ws.name` = workspace-root basename (`cargo.rs:98`) | checkout directory name | compared; fixed by the workflow checkout layout |
| `show-env:LLVM_PROFILE_FILE` whole name | `LLVM_PROFILE_FILE_NAME` env (`main.rs:210`) | an explicit override | compared; a real difference |
| `show-env:CARGO_LLVM_COV_TARGET_DIR`, `_BUILD_DIR` | Cargo metadata | target path, `build.build-dir` | neutralised when equal to the target; otherwise compared |
| `show-env:RUSTC_WRAPPER` | `cx.current_exe` (`wrapper.rs:84`) | tool install path | compared; the same per runner image |
| `show-env:__CARGO_LLVM_COV_RUSTC_WRAPPER_RUSTFLAGS` | rustc version, target, flags (`main.rs:153-205`) | toolchain | compared; the receipt also binds rustc |
| `show-env:__CARGO_LLVM_COV_RUSTC_WRAPPER_CRATE_NAMES` | workspace members and targets (`wrapper.rs:63-83`) | source | compared |
| `show-env:__CARGO_LLVM_COV_RUSTC_WRAPPER_PRE_EXISTING` | configured `build.rustc-wrapper` (`wrapper.rs:85-89`) | local build-cache configuration | compared; a real build difference |
| `show-env:__CARGO_LLVM_COV_RUSTC_WRAPPER`, `CARGO_LLVM_COV`, `CARGO_LLVM_COV_SHOW_ENV` | constants | nothing | compared |
| `env:*` (`PROFILE_ENV`) | process environment | CI and task configuration | compared; none is credential-bearing |

The CPU count is the only value that varies by hardware between runners of one image.

## Goals / Non-Goals

**Goals:** partitions built identically on hosts of different sizes agree. Every other difference is still refused. A future disagreement names its key from uploaded evidence.

**Non-Goals:** changing the named variables, the neutralisation of the workspace-name stem or the wrapper path, the receipt fields or their order, the pinned cargo-llvm-cov version, or the workflow. The incidental `KURU_TEST_SUPERVISOR_PREPARED = false` observation (below) is also out of scope.

## Decisions

- **Replace only the size, by structure.** In the file-name component of `LLVM_PROFILE_FILE` (after the last `/` or `\`), each `%` followed by one or more ASCII digits and then `m` has its digits replaced with `${KURU_COVERAGE_POOL}`. A size-less `%m`, `%p`, `%h` and the other specifiers are kept, and so is any `%<N>m` inside a directory component, so `-%m` and `-%3m` still differ. The rule applies only to that key. Rejected alternatives: stripping the specifier, which would equate pool and non-pool patterns; a regex over every value, which could hide a real difference; and passing `LLVM_PROFILE_FILE_NAME` to cargo-llvm-cov, which would change the profile pattern the tests actually use, a behavior change beyond this fix.
- **Persist the map as a separate evidence file instead of a receipt field.** `profile-env.json` is the `ProfileEnv` map written like the other JSON evidence, and the receipt's `profile_env_sha256` is `digest_json` of that map, so the receipt already binds it. Adding a `Receipt` field would duplicate the digest and change a `deny_unknown_fields` schema with no in-version precedent, and so would require a `SCHEMA` bump for every evidence document. The file adds one entry to the exact evidence layout. The merge refuses a missing file (`unexpected artifact entries`) or one that does not digest to the receipt value, so the map cannot drift from what was compared.
- **Name keys in the existing error.** The agreement error keeps its form and appends `; <key>: <partition value> != <partition 1 value>` for each differing key. Absent keys print as `<absent>`, and values over 160 characters are truncated with their byte length, because the crate-name list is kilobytes long.

## Risks / Trade-offs

- [A host-dependent value not yet seen] → it would now be named by key in the merge error and in the uploaded evidence, and could be neutralised by structure in a later change.
- [Evidence from the old layout] → it is refused, which is harmless because the partitions and merge of one run share a commit. A rerun of an old run's merge job uses that run's code.
- [`KURU_TEST_SUPERVISOR_PREPARED`] → the reconstruction matched only with this variable `<unset>`, although `packages/kuru-delivery/mise.toml` sets `KURU_TEST_SUPERVISOR_PREPARED = false` for `coverage:shard`. This is consistent with mise treating a boolean `false` env value as unset. It is recorded for a separate follow-up and is not changed here, since changing it would alter test behavior and the digest.
- [Only CI can prove] → a hosted macOS merge across runners that actually report different CPU counts. The local tests prove the digests are equal for `%3m`, `%5m`, `%14m` and `%128m`, and the reconstruction proves this was the only differing value in run 36426742472.

## Operational surface

No workflow, runner, secret or binary change. The evidence artifact of each partition gains `profile-env.json`. The uploads already take the whole evidence directory (`native-tests.yml`, `path: ${{ runner.temp }}/kuru-coverage-evidence-…`).

## Integration contract

The fix depends on cargo-llvm-cov 0.9.1's `show-env` output shape for `LLVM_PROFILE_FILE`. A tool bump that changes the pattern would still be compared as written, and any new difference would be named by key.
