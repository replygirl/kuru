## Context

#111 runs each OS's coverage as five package shards through `kuru-delivery coverage shard`. Every shard compiles the
full instrumented inventory into a fresh `runner.temp` target. Its Cargo runner (`coverage dispatch`) runs the shard's
packages and records the rest as `omit`, and a same-OS `coverage collect` rebuilds the inventory, validates the five
receipts and runs one `llvm-cov report --fail-under-lines 90`. The measured floors (#111 run 36270435157) are the
indivisible executables (`kuru_runtime` 711 s on Ubuntu and 1258 s on Windows, `kuru_memory` 614 s on macOS), a cold
dependency window in every job (123 / ~185 / 258 s), and a serial collect (458 / 291 / 585 s) that on macOS competes
for the account-wide five-job cap. The Linux arm64 `native-build` runs the uninstrumented `kuru-memory` suite
serially (628 s). cargo-llvm-cov 0.9.1 instruments only workspace crates. The ~430 dependency crates are ordinary
rlibs.

Constraints that bind the approach: AGENTS keeps per-OS 90% gates, one coverage writer per fresh target, the
instrumented supervisor for coverage (`KURU_TEST_SUPERVISOR_PREPARED=false`) and the prepared snapshot for ordinary
real-memory tests. It also requires native checks on every supported OS, pins by exact version and SHA, and no
parallel task workspace. The lead's decisions (2026-09-26/27) are: keep every test; keep macOS instrumented; keep
per-OS gates; approve the Ubuntu merge with receipt agreement on the conditions in the spec; use N = 4 on macOS, 8 on
Windows and 8-12 on Ubuntu; and run the arm64 suite through uninstrumented partitions.

## Goals / Non-Goals

**Goals:**
- Divide test time within executables, not between packages, while keeping exactly-once evidence.
- Remove each OS's instrumented rebuild from the critical path and the macOS collect from the macOS cap.
- Warm the dependency window without weakening the fresh target or the receipts.
- Make each cache's contribution visible per job, and make eviction cost time, never correctness.

**Non-Goals:**
- Duration-aware bins, test relocation to Ubuntu only, a union-LCOV gate and dropping macOS instrumentation (the
  lead reversed or deferred each).
- Warm Dolt reuse in test support (runtime-owned) and the cold-open fixture (PR-B).
- Changing `bundle:verify-native-build` (see D9), `RUST_TEST_THREADS`, or `mise run coverage` for local use.
- Product crates. Only `kuru-delivery` tooling, workflows and docs change.

## Decisions

**D1. Partition by hashed test identity inside the existing runner.** The dispatcher runs
`<exe> --list --format terse` and parses only `<name>: test` lines, failing on anything else, including `: bench`
and duplicates. It assigns a test to partition `1 + (u64_be(sha256("sha256-artifact-test-v1\0" + package/kind/target + "\0" + name)[0..8]) % N)`.
It hashes the artifact identity, not the executable path, because the path carries Cargo's metadata hash and `.exe`.
Ignored tests are assigned like others and remain ignored. Rejected: `--partition hash:k/N` from cargo-nextest, which
replaces the checked libtest runner, ledger and supervision and adds a pin. Also rejected: package or executable
bins, which cannot divide a 1258 s executable.

**D2. `--exact` chunks under one cross-OS budget.** Assigned names are chunked so each invocation stays under 30,000
UTF-16 units, measured with Windows `CreateProcess` quoting on every OS. Chunk boundaries are therefore identical and
testable anywhere, and the 32,767 limit keeps 2,767 units of margin. A name that cannot fit alone fails the partition.
The deadline is re-checked before each chunk. `LibtestProgress` records the `running N tests` count, which must equal
the chunk size. Rejected: a Unix-only budget (non-portable evidence) and a response file (libtest has none).

**D3. Receipt schema 2 and the ledger seams.** `SCHEMA` becomes 2, and v1 files are refused by name. The receipt
carries the partition `{scheme, index, count}`, `mode`, `scope`, the #111 identity fields, `profile_env_sha256`
(sorted `CARGO_INCREMENTAL`, `CARGO_PROFILE_{DEV,TEST}_DEBUG`, `RUST_TEST_THREADS`, `KURU_TEST_SUPERVISOR_PREPARED`
and, when instrumented, the parsed `show-env` map), `inventory_sha256`, `plan_sha256`, `tests_sha256` (the
(artifact, list hash) pairs), ledger digests, a profile summary `{count, bytes, manifest_sha256}` and, when
instrumented, `lcov {bytes, sha256, files, lines_found, lines_hit}`. `partition-plan.json` stores each executable's
full listed and assigned names (about 90 KB). Cargo still invokes each executable once. Cargo-supplied arguments must
stay empty. `RunnerRecord` gains `artifact`, `list_sha256`, `listed`, `assigned_sha256`, `invocations[{kind,
names, args_sha256, announced, started, finished, status_code}]`, `started`/`finished` and profile counts before and
after. `validate_run_ledger` recomputes each assignment from the recorded list and requires that the chunks'
disjoint union equals it. `Selection` and `SHARDS` are removed. `WORKSPACE_PACKAGES` and `PARTITIONS: [(os label,
mode, count)]` replace them.

**D4. Per-partition LCOV and an Ubuntu merge (lead decision: receipt agreement replaces the independent rebuild).**
`llvm-cov report` needs the instrumented objects, which an Ubuntu runner lacks for macOS and Windows. Each instrumented
partition therefore checks its profile count against `PROFILE_COUNT_LIMIT` and exports
`llvm-cov report --failure-mode any --lcov` against its own binaries, with no threshold. It rewrites `SF:` to
root-relative forward-slash paths, refusing any source outside the root, then hashes and uploads the LCOV. Raw
profiles stay on the runner. Every partition runs `--list` for every executable, so every object has profile data and
`--failure-mode any` never meets an unprofiled object. The merge (`coverage merge`, `runs-on: ubuntu-latest`, one job
per OS) downloads `<prefix>-coverage-<os>-partition-*` with one step. Diagnostics are renamed
`<prefix>-coverage-diagnostics-<os>-…` so that pattern cannot match them. The merge takes each index's latest attempt
no later than the run attempt and applies the spec's agreement, completeness and LCOV-identity checks. It unions DA and
FNDA counts, recomputes LF/LH, enforces Σ LH / Σ LF ≥ 0.90, and writes a temp file that it renames only after
passing. It does not compare the Ubuntu host's `identity()`, which says nothing about another OS. Instead it requires
`target_os`/`target` consistent with the label and `source` equal to the expected commit. Rejected: (a) uploading
instrumented executables for an off-OS `llvm-cov report`, which needs cross-object reading plus path remapping,
unsupported by cargo-llvm-cov, and large artifacts; (b) keeping a same-OS rebuild collect, which keeps 291-585 s on
the critical path and a macOS slot; (c) a build-once job, which adds a serial build and saves no wall clock.

**D5. Seeded dependency cache with an orchestrator-owned allow-list.** `prepare` becomes: fresh target and state →
`show-env` → `cargo metadata --no-deps` → **seed import** → `cargo test --no-run`. Import moves (or copies across
devices) only `<seed>/debug/{deps,build,.fingerprint}/<stem>-<16 hex>[.ext]` whose stem is not a workspace package or
crate name. It refuses and counts symlinks or reparse points, `*.profraw`, `kuru-shard-state`,
`kuru-test-supervisors`, other seed-root entries and non-regular files. On `main`, partition 1 alone exports after its
receipt is written, using the same allow-list, so the cache holds exactly what an import accepts.
`actions/cache/restore` and `actions/cache/save` (current release, pinned by SHA) use
`kuru-coverage-seed-v1-<os>-<mode>-hashFiles(Cargo.lock, mise.toml, mise.lock)` with a prefix restore key. Only main
saves. The target is one fixed path per job (`$RUNNER_TEMP/kuru-coverage-target`), so embedded build-script and
dep-info paths match. The fresh-target check still refuses an existing tree. Warm efficacy is measured exactly, not
from logs: the number of non-workspace `compiler-artifact` messages with `"fresh": false`. Rejected: a second
rust-cache step with a relative `workspaces` path (#107's approach relies on undocumented cleanup of a nested target)
and sccache (it cannot cache proc-macros or build scripts and adds a trust boundary).

**D6. Uninstrumented mode is the same subcommand with a flag.** `coverage shard --uninstrumented` requires
`KURU_COVERAGE_PACKAGES` as the scope. Its inventory is the `cargo test -p <scope> --all-targets --all-features
--locked --no-run` artifact set (`artifact_inventory` gains the scope; instrumented passes the whole workspace). There
is no `show-env` and no LCOV, and `instrumentation` is a distinct contract string. Because `test_support.rs` resolves
the prepared supervisor from `current_exe()` as `<target>/<profile>/kuru-test-supervisors`, the orchestrator runs
`cargo run -p kuru-memory --features test-support --locked -- prefetch` with `CARGO_TARGET_DIR=<fresh target>`
before the tests. The task `test:partition` sets `KURU_TEST_SUPERVISOR_PREPARED=1`, as `kuru-memory:test` does.
The merge takes an expected mode and refuses a receipt of the other mode.

**D7. Counts and concurrency.** `PARTITIONS` = ubuntu-latest 8, macos-latest 4, windows-latest 8 (instrumented),
ubuntu-24.04-arm 3 (uninstrumented, `kuru-memory`). `artifact_os_label` (#111: `[a-z0-9-]+`) is widened to admit `.` for `ubuntu-24.04-arm`; PR6b's `windows-11-arm` already fits. The model is F + 1.05 Σ/N plus a ~180 s Ubuntu merge, with F =
224 / 164 / 246 s (warm seed) or 346 / 304 / 501 s (evicted). The resulting paths are Ubuntu 651 s (10.9 min;
evicted 12.9), macOS 820 s (13.7; 16.0) and Windows 918 s (15.3; 19.6). arm64 is estimated at about 9-10.5 min, with
no uninstrumented partition timed yet. The run-level floor is about 15.3 min, set by Windows (today 43.9). macOS:
four partitions plus the install job fill the five-job cap at t = 0, so nothing queues within the run. N = 5 would
queue install behind a partition and push macOS to 20-24 min. At t = 0 the run starts 8 + 4 + 8 + 3 partitions,
3 installs, 8 quality, native-build and native-platform, which is 36 of the 40 cap assumed from #111 (at least 22
concurrent non-macOS starts, so at least Pro). Ubuntu 12 would reach 40 with no slack while Ubuntu is not the
critical path, so Ubuntu uses 8. `native-tests.yml` receives `os` as an input, so the matrix is
`partition: ${{ fromJSON(inputs.os == 'macos-latest' && '[1,2,3,4]' || '[1,2,3,4,5,6,7,8]') }}`.
`release_workflow.rs` pins that expression against `PARTITIONS`. The partition timeout is 45 min (deadline at 35) and
the merge timeout is 20.

**D8. Visibility without new failure modes.** Each partition writes `job-ledger.json` with phase timestamps, cache
state (helper and seed `hit | partial | miss | absent | unknown`, the matched key, imported and refused counts,
dependency and workspace units rebuilt), a profile summary and per-executable timings, invocation counts and profile
counts. The workflow passes the cache-hit outputs as `KURU_COVERAGE_*_CACHE` env. An empty value records `unknown`.
The merge prints a per-partition table and uploads `merge-summary.json`. Only a malformed ledger that the partition
itself wrote can fail a job.

**D9. `bundle:verify-native-build` stays in the Windows install job.** It is three offline `cargo build -p kuru
--release` checks of the memory build-script boundary. It has no libtest inventory, so the lead's "run it through
uninstrumented partitions" has no referent. Options for the lead, none implemented: (a) leave it (recommended; it sits
in the install job, not the coverage path); (b) give it its own parallel Windows job with its own release build;
(c) ask kuru-memory's owner whether the valid-archive build can be `-p kuru-memory`.

**D10. PR6b table coordination.** `EXCLUDED_ARTIFACTS: [(host target, artifact, reason); 0]` is consumed now. An
excluded executable is still listed. It is recorded as `exclude` with its reason in the plan and receipt and
subtracted from the run set. The merge requires the same reason in every partition, and a stale entry fails. PR6b then
adds only its row and a `("windows-11-arm", Uninstrumented, N)` partition row. New logic lives in new
`coverage/*.rs` modules so `coverage.rs` edits stay limited to types, seams and tables. Whichever PR lands second
rewrites the other's matrix assertion (`SHARDS` → `PARTITIONS`).

**D11. Injectable boundaries.** `orchestrate::Host` gains no new methods: the LCOV export and prefetch go through
`stream`. A new `coverage::Launcher` trait (`list`, `run`) wraps today's Unix `supervise_group` and Windows
`NativeSpawnSpec` paths unchanged, and `dispatch_test` becomes `dispatch_with(launcher, …)`. `partition`, `lcov`,
`merge`, `seed` and `ledger` are pure modules over data or temp trees. Live proof uses the test binary itself: a unit
test runs `current_exe() --list --format terse` and `--exact` over two no-op probe tests. A `#[cfg(windows)]` test
spawns a budget-sized chunk.

## Operational surface

- Runners (GitHub-hosted, no containers): `ubuntu-latest` (x86_64, 4 vCPU), `macos-latest` (arm64, 3 vCPU,
  account-wide five-concurrent-job cap), `windows-latest` (x86_64, 4 vCPU) and `ubuntu-24.04-arm` (aarch64,
  4 vCPU) for partitions; every merge runs on `ubuntu-latest`. No bind address or network listener is added.
- Concurrency: 36 jobs at t = 0 against the assumed 40-job account cap; merges start after their partitions.
- Secrets: none added. `GITHUB_TOKEN` is used only by the existing mise-action step; cache and artifact steps use
  the workflow's own token scope (`contents: read`, default actions cache/artifact access).
- Pinned binaries: Rust 1.98.1, cargo-llvm-cov 0.9.1 (instrumented partitions only), mise 2026.9.4 and the existing
  action SHAs; `actions/cache/restore` and `actions/cache/save` are added at their current release, pinned by SHA.
- Storage: one seed cache entry per OS and mode (~1.0-1.4 GB each; repository limit raised to 200 GB); evidence
  artifacts per partition carry LCOV, plan, ledgers and receipt instead of raw profiles.
- Timeouts: partitions 45 min with the inner test deadline at 35 min; merges 20 min.

## Risks / Trade-offs

- [Rigor: no independent rebuild] → Every receipt must agree field by field and the lists must be provably complete.
  Each partition's LCOV is computed against the binaries that produced its profiles. M runs an equivalence drill by hand on
  one machine and SHA: all partitions' raw profiles fed to one rebuilt `llvm-cov report` (the #111 collect recipe)
  and the DA-merge of their LCOVs must give identical DA sets and LF/LH.
- [Own threshold arithmetic replaces `--fail-under-lines`] → The same drill compares totals with cargo-llvm-cov's
  summary. Branch records are refused, and no percentage is averaged.
- [Count-balanced hash assignment is uneven in time (largest single test 145 s on Windows)] → M records max/mean
  partition test wall per OS. Duration bins are a follow-on using the same pure function with a committed weights
  file.
- [Seeded units not fresh under the cargo-llvm-cov `RUSTC_WRAPPER` (unverified)] → The worst case is no speedup. M
  records rebuilt dependency units warm versus evicted, and the target is 0 when warm.
- [Profile growth: list runs and chunks add one profile per process] → Profile limits are checked per partition
  before export and profile counts are logged. N×executables no longer aggregates, since the merge reads LCOV.
- [Artifact-name glob collisions] → Diagnostics are renamed so they cannot match the evidence pattern, and the Rust
  layout parser refuses foreign labels and unknown indices.
- [Concurrency: other runs share the 40-job and five-macOS-job caps] → Counts leave four general slots. macOS
  queueing across runs is outside this change and remains visible in job start times.
- [Transition while #111 is unmerged] → The branch is stacked on #111's head and cherry-picked onto `main` after it
  merges. The receipt schema bump prevents mixing v1 and v2 evidence across a rerun that spans the change.
