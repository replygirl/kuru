## Why

Native coverage is the longest part of CI: in the green #111 run the critical path was 26.4 min on Ubuntu,
58.1 min on macOS (22.4 min without queueing) and 43.9 min on Windows. The longest package shards are single test
executables (`kuru_runtime` 711 s on Ubuntu, 1258 s on Windows), so adding package shards cannot shorten them.
Every shard also compiles its dependencies cold. Each OS's collect job then rebuilds the instrumented inventory
on that OS after the slowest shard, and on macOS it takes one of the five concurrent macOS job slots. The Linux arm64
`native-build` job runs the full uninstrumented `kuru-memory` suite serially (628 of its 1002 s) and sets its
own run-level floor. This change shortens those paths while keeping every test, the per-OS 90% line gates and
fail-closed evidence.

## What Changes

- Per-test partitions inside the Rust coverage orchestrator. Each partition still compiles the full
  workspace/all-targets/all-features inventory. For every test executable it runs `--list`, assigns each test to
  one of N partitions with a stable hash of the executable's artifact identity and the test name, and runs the
  partition's tests with explicit `--exact` names. The names are chunked so no command line reaches Windows' 32,767
  character limit, and libtest's announced count must equal each chunk. Package shards and `SHARDS` are replaced by
  a per-OS partition table.
- Receipt schema 2. Each receipt records the partition index and count, source SHA, tree, Cargo.lock, rustc and
  cargo identity, cargo-llvm-cov version, a digest of the profile environment and the inventory hash. It also records
  each executable's listed and assigned test names, stored in an uploaded partition plan and hashed into the receipt.
  An instrumented partition exports its own LCOV, normalized to root-relative paths and hashed into the receipt.
  Raw profiles are no longer uploaded.
- The merge job runs on `ubuntu-latest` for every OS. It replaces the per-OS collect job that rebuilt the
  inventory. The merge accepts an OS only when every partition receipt is present, all receipts are identical on
  source, toolchain, profile environment and inventory hash, and every executable's assigned test sets are pairwise
  disjoint with a union equal to its `--list` output. Any mismatch or missing receipt fails the merge before any
  report exists. The merge then unions the LCOV line records, writes that OS's LCOV and enforces at least 90% line
  coverage once per OS by cargo-llvm-cov's summary metric, reproduced exactly from per-instantiation line sets
  (design D4a). **BREAKING (CI contract):** the fail-closed check is now N independent partition builds that
  agree with each other. The merge job does not rebuild the inventory. This is an explicit lead decision recorded in
  the design.
- Seeded dependency cache. A partition still creates a fresh target at one fixed per-job path. Before its first
  build it imports allow-listed non-workspace dependency artifacts from a restored cache directory under
  `runner.temp`. It never imports workspace-crate artifacts, profiles or private supervisor or state directories.
  Only `main` exports and saves the seed. Eviction or a miss makes the run slower but never fails it, and the
  receipts are unchanged.
- Uninstrumented mode (`coverage shard --uninstrumented`, task `test:partition`). It uses the same partitions,
  receipts, completeness and agreement checks, with no instrumentation. It runs the Linux arm64 `kuru-memory` suite
  in partitions with its own Ubuntu merge. `native-build` keeps the release build, packaging and the embedded-runtime
  check. Windows `bundle:verify-native-build` stays where it is. It runs three offline `cargo build` checks and has no
  test inventory to partition. The lead chose to leave it in the Windows install job (design D9, option (a)).
- Shard counts: Ubuntu 8, macOS 4 (four partitions plus the install job fill the five-job macOS cap; the merge no
  longer uses a macOS slot), Windows 8 and arm64 memory 3. The design gives the arithmetic.
- Ledgers: the runner ledger records per-executable start and finish timestamps, invocations and profile counts. A
  per-partition job ledger records phase timestamps, helper and seed cache hit or miss, imported and refused seed
  entries, the number of rebuilt dependency units and profile totals. The merge prints them per partition.
- `docs/development.md` describes the partition topology, the agreement rule ("N independent shard builds agreeing
  rather than a separate rebuild"), manual partition and merge runs, and the seed cache.

## Capabilities

### New Capabilities

### Modified Capabilities
- `repository-delivery`: adds requirements for exactly-once partitioned native test evidence, merged by receipt
  agreement with a per-OS coverage gate, and for the seeded dependency cache that never affects evidence.

## Impact

- Code (repository tooling only, no product crate): `packages/kuru-delivery/src/coverage.rs`,
  `packages/kuru-delivery/src/coverage/orchestrate.rs`, new `coverage/{partition,lcov,merge,seed,ledger}.rs`,
  `packages/kuru-delivery/src/main.rs` (`coverage shard [--uninstrumented]` and `coverage merge`, replacing
  `coverage collect`), `packages/kuru-delivery/mise.toml` (`coverage:shard`, `coverage:merge`, `test:partition`).
- Workflows: `.github/workflows/native-tests.yml` (partition matrix, fixed target, seed restore/save, artifact names,
  Ubuntu merge job) and `.github/workflows/ci.yml` (`native-build` without the memory suite, `native-memory`
  partitions and merge, `ci-gate` needs). `packages/kuru-delivery/tests/release_workflow.rs` pins them.
- Artifacts: `<prefix>-coverage-<os>-partition-<k>-attempt-<n>` evidence,
  `<prefix>-coverage-diagnostics-<os>-partition-<k>-attempt-<n>` diagnostics and the unchanged merged
  `<prefix>-coverage-<os>-attempt-<n>` LCOV.
- Docs: `docs/development.md`. `AGENTS.md` keeps its gate wording. Task 8 re-reads it and records whether any line
  changes.
- Stacked on #111 (`ci/coverage-shard-orchestrator`, archived in this tree as `coverage-shard-orchestrator`). The
  branch moves onto `main` after #111 merges. PR6b (`feat/windows-arm64`) coordinates the `coverage.rs` tables, as
  the design explains.
- Cache: one seed entry per OS and mode, about 1.0 to 1.4 GB each. The maintainer raised the repository cache limit
  to 200 GB.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
