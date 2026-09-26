## 1. Shard list and OS-labelled collection (T1)

- [x] 1.1 Derive the shard workspace package list from `SHARDS` in `coverage.rs` (replacing the list hardcoded in `windows-coverage.ps1`) and add a unit test that `SHARDS` covers exactly the eight workspace packages once, and verify with `mise run //packages/kuru-delivery:test`
- [x] 1.2 Thread an `artifact_os` label (validated `[a-z0-9-]+`) from `KURU_COVERAGE_OS` through `collect_profiles` so the marker becomes `-coverage-<os>-<shard>-attempt-`, update the aggregate fixtures, add a test that collect rejects an artifact labelled for another OS, and verify with `mise run //packages/kuru-delivery:test`

## 2. Unix test supervision (T2)

- [x] 2.1 Read `take_transition` in `packages/kuru-platform/src/unix.rs` for the already-exited root case and confirm the `tokio` features `kuru-delivery` needs (Unix pipe receiver) before any `Cargo.toml`/`Cargo.lock` change, and verify by recording both findings in the change evidence
- [x] 2.2 Replace the `#[cfg(not(windows))]` `Command::status()` dispatch with a `TestProcess` over `OwnedProcessGroup` (piped stdout relayed through the unchanged `supervise`, deadline wait, sample, `terminate_before_reap`, bounded `reap_if_exited`, then `presence_after_reap` in the stall report; normal exit through the same terminate/reap path), forward the full child environment including `LLVM_PROFILE_FILE`, remove the `not(windows)` dead-code allowances, and verify with `mise run //packages/kuru-delivery:test`
- [ ] 2.3 Add `#[cfg(unix)]` tests: a fake process for deadline/sample/terminate ordering, and a real stalled child with a grandchild proving group termination, reap before presence, and a `.stall.json` recording `presence_after_reap`, and verify with `mise run //packages/kuru-delivery:test` on macOS and Linux

## 3. Rust shard/collect orchestrator (T3)

- [x] 3.1 Add `packages/kuru-delivery/src/coverage/orchestrate.rs` with `CoverageCommand::Shard`/`Collect` in `main.rs`: per-mode `KURU_COVERAGE_*` input validation, fresh diagnostics directory created first with `failure.txt` plus manifest/ledger copies on any later error, absolute canonical root, fresh target with `kuru-shard-state`, helper = `current_exe()`, pinned `cargo-llvm-cov` version check, `verify_source`, and a streaming cargo helper without the 64 KiB/30 s bound and without a compile deadline, and verify with `mise run //packages/kuru-delivery:test`
- [x] 3.2 Put every cargo/cargo-llvm-cov/rustc invocation behind an injectable boundary so shard and collect sequencing (inventory, selection, runner config, profile discard, test run, ledger validation, receipt; collect profiles, existing-report refusal, `report --failure-mode any --fail-under-lines 90 --lcov`) and each failure path are unit-tested with a fake runner, and verify with `mise run //packages/kuru-delivery:test` and `mise run coverage` holding the 90% workspace line gate
- [ ] 3.3 Add a strict `show-env` parser (`NAME="value"`, `NAME` in `[A-Z_][A-Z0-9_]*`, only cargo-llvm-cov 0.9.1 escapes, any other line fails) applied only to the cargo child environment map, with captured Linux, macOS and Windows `show-env` fixtures and an unknown-escape rejection test, and verify with `mise run //packages/kuru-delivery:test`

## 4. Tasks and retired PowerShell (T4)

- [x] 4.1 Replace `coverage:windows:shard`/`:collect` with OS-agnostic `coverage:shard`/`coverage:collect` (`cargo run -p kuru-delivery --features tooling --locked --bin kuru-delivery -- coverage shard|collect`, `dir = "{{config_root}}/../.."`, same `depends`, `tools` and `[env]` incl. `KURU_TEST_SUPERVISOR_PREPARED = false`, no `run_windows`), delete `support/windows-coverage.ps1`, keep `coverage:workspace`, and verify with `mise tasks` listing and `mise run lint:tooling`
- [x] 4.2 Replace the ps1 text assertions in `tests/powershell_diagnostics.rs` with orchestrator-facing checks and retarget the two `#[cfg(windows)]` cmd-launch tests to the new run string, and verify with `mise run //packages/kuru-delivery:test` (Windows cases on the native Windows shard)

## 5. Workflows (T5, after rebasing onto PR5)

- [x] 5.1 In `native-tests.yml` run the five-row `SHARDS` matrix on every OS (job start recorded first, `runner.temp` bundle/target/evidence/diagnostics dirs, Linux space reservation and Secret Service session behind `runner.os`, `KURU_COVERAGE_JOB_MINUTES` equal to `timeout-minutes`, receipts on success, diagnostics on failure), one `native-coverage-${{ inputs.os }}` rust-cache key with `save-if` on `connectors-core-platform`, and `KURU_MBX=0`, and verify with `mise run lint:tooling`
- [x] 5.2 Add one collect job per OS that refuses unless every shard succeeded, downloads five per-shard patterns into `inputs/<shard>`, repeats `CARGO_PROFILE_TEST_DEBUG` (`0` on Linux, else `line-tables-only`) exactly as the shard step, enforces `--fail-under-lines 90` once and uploads `<prefix>-coverage-<os>-attempt-<n>`; remove the Unix monolithic `coverage` job and keep PR5's install job; update `native-gate`, and verify with `mise run lint:tooling`
- [x] 5.3 Update `tests/release_workflow.rs` (matrix equals `SHARDS` on every OS, five downloads, shared key, job-start step, per-OS artifact names, identical profile debug env in shard and collect, gate table), and verify with `mise run //packages/kuru-delivery:test`

## 6. Documentation (T6)

- [x] 6.1 Update `docs/development.md` (five shards and one report on every OS, Job on Windows or process group on Unix, `<prefix>-coverage-<os>-<shard>-…` names, `coverage:shard`/`coverage:collect` rows, shard targets in `runner.temp` never cached, a local orchestrator recipe against a throwaway target), and verify with `mise run docs:check`

## 7. Local verification set

- [x] 7.1 Run `mise run format:code ::: lint:rust ::: typecheck ::: lint:tooling ::: cospec:validate ::: cospec:managed:check ::: docs:check`, then `mise run //packages/kuru-delivery:test` and `mise run coverage`, and verify each exits 0 with the observed output recorded in `verification.md`

## 8. Hosted measurement and stall drill

- [ ] 8.1 (M) From the first green CI run on each OS, record per-shard wall-clock and the compile/test split (runner-ledger first record against job start) in `verification.md`, and verify the numbers cite the run and job IDs
- [ ] 8.2 (D) Push a throwaway draft commit adding a never-ending test to `kuru-core`, and verify on Linux, macOS and Windows that `connectors-core-platform` fails at its deadline (`timeout-minutes` less the 10-minute reserve) with a `…-connectors-core-platform-diagnostics-attempt-<n>` artifact containing a stall report, not a host cancellation; then drop the commit
