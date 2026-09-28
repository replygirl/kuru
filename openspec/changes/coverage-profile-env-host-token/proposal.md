# Proposal

## Why

The partitioned coverage merge refuses partitions whose receipts disagree, and the profile-environment digest in each receipt included a value that depends on the runner's hardware. cargo-llvm-cov 0.9.1's `show-env` always takes its nextest branch (`src/main.rs:73`, `set_env(cx, writer, IsNextest(true))`) and writes `LLVM_PROFILE_FILE` as `<target>/<workspace>-%p-%<N>m.profraw` with `N = std::thread::available_parallelism()` (`src/main.rs:207-246`); the orchestrator's `profile_env_sha256` neutralised only the target path. Two `macos-latest` runners with different CPU counts therefore produced different digests for identical builds, and run 36426742472 (PR #125) failed its macOS merge with `coverage partition 2 receipt differs from partition 1 in profile environment` although every other receipt field agreed and every test passed. The same minority digest appeared in run 36424722859 (PR #126). Rebuilding the neutralised map from the source tree at `95c79127` reproduces both CI digests exactly: `N=3` gives the majority `46e05ee6…` and `N=5` the minority `0a632ea0…`, with every other value unchanged. The refusal also named only two digests, because the digested values were never persisted, so the differing key could not be read from any artifact.

## What Changes

- Before digesting, the orchestrator replaces the size `N` of each `%<digits>m` merge-pool specifier in the file name of `show-env`'s `LLVM_PROFILE_FILE` with the fixed token `${KURU_COVERAGE_POOL}`, beside the existing `${KURU_COVERAGE_TARGET}` target-path token. Directories, a size-less `%m`, and every other specifier, stem, extension and variable are still compared as written. The children still receive show-env's real pattern.
- Each partition uploads the neutralised key/value map it digests as `profile-env.json` in its evidence directory. The receipt's `profile_env_sha256` is the digest of that map, so the receipt schema and version are unchanged. The map contains Cargo and test settings plus cargo-llvm-cov's paths, flags and crate names. It holds no credentials.
- The merge requires `profile-env.json` in every partition's evidence, refuses one whose digest differs from its receipt, and on a profile-environment disagreement names every differing key with both values, bounded.
- Corrects the lines.rs comment and development docs: a negative counter expression comes from instrumented code still running on a detached thread while the process writes its profile at exit, as observed in PR #125. It does not come from lost non-atomic counter updates, because counter updates are atomic. The refusal behavior is unchanged.
- Not changed: the fields compared, their order, the other receipt fields, the named process variables, the target-path neutralisation, and the refusal of any genuine difference.

## Capabilities

### New Capabilities

### Modified Capabilities

- `repository-delivery`: the receipt-agreement requirement now states what the profile-environment digest neutralises (target path and host merge-pool size only), that the digested environment is uploaded and verified against the receipt, and that a disagreement names the differing keys.

## Impact

- `packages/kuru-delivery/src/coverage/orchestrate.rs`: `digested_profile_env` (was `profile_env_sha256`), `neutral_pool`, `POOL_TOKEN`; `Prepared` carries the map.
- `packages/kuru-delivery/src/coverage.rs`: `ProfileEnv`, `PROFILE_ENV_FILE`; `ReceiptOptions::profile_env` replaces `profile_env_sha256`; `write_evidence` writes the map and derives the digest from it.
- `packages/kuru-delivery/src/coverage/merge.rs`: evidence entry list, digest check, `differing_keys` in the agreement error.
- `packages/kuru-delivery/src/coverage/fixture.rs`, `lines.rs` (comment only), `docs/development.md`.
- The evidence layout gains one file per partition. Partitions and the merge of one run come from the same commit, so they stay mutually consistent. Evidence from before this change is refused as an unexpected entry set, the same way any other layout change is refused.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
