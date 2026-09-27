# Development

Install pinned tooling with `mise install`, then run `mise run setup`. Rust 1.98.1
is declared in both mise and rust-toolchain.toml. Cargo.lock pins runtime
transitives. The [dependency audit](dependencies.md) records latest stable
versions and the exact upstream constraints on transitive updates. mise.lock contains platform-specific tool URLs and checksums.
Cospec is a standalone executable with embedded OpenSpec. Its validate/apply
JSON and managed-file checks run without a project OpenSpec dependency. The
pinned 0.8.2 release still needs the compatibility fix for its embedded
OpenSpec 1.13.1 bundle: duplicate entrypoint execution makes the unpatched
instructions command fail to return one JSON document. The cospec mise task scopes a small
[compatibility preload](../packages/kuru-delivery/support/cospec-preload.cjs) to
that exact bundle hash using cospec's own runtime. It preserves command arguments
and the original gate; standalone contract tests cover clear, hard-blocked,
soft-blocked and missing-artifact outcomes. Remove the preload only after an
upstream release fixes vendoring and passes those tests without it. The 0.8.x
OpenSpec 1.13.1 update and archive-gate corrections do not satisfy that removal
condition; without the preload, 0.8.2's apply gate cannot parse the embedded
instructions output.

The architecture follows this order: apps/ and packages/ ownership, mise
monorepo tasks, Rust, then other tools. Every app/package owns a mise.toml;
root aliases use `//path:task` addresses. For example,
`mise run //apps/kuru-tui:test` works from any directory, and `mise run test`
inside that app runs its tests. Cargo's workspace shares dependency resolution
and a single coverage report. It does not replace mise task ownership.

Installation, packaging, release orchestration and repository checks live in
the Rust package `packages/kuru-delivery`. No Python or Bun is needed. The
VitePress docs app owns its Node/npm pins, package.json and npm lockfile under
`apps/kuru-docs`; its mise tasks invoke the installed tools directly.
The root npm backend alias makes monorepo task discovery use that app's locked
`npm:npm` backend. Versions remain app-owned. Documentation setup checks actual
Node/npm versions against `mise current` even when `npm ci` is already cached;
a runner's bundled npm must not silently replace the configured version.
If an earlier local install used another backend for the same npm version,
repair only that cache with `mise -C apps/kuru-docs install --force npm`.

The delivery package activates Cocogitto and Communiqué only for its tests,
combined coverage and release tasks. Its `setup` task preinstalls those tools
with mise's `--include-task-tools` option; lean CI jobs use `setup:test-tools`
to install only those two exact package-owned pins. Communiqué 1.4.2 provides Linux x86_64
and arm64, macOS arm64 and Windows x86_64 binaries, which cover every supported
platform for the full maintainer gate.

CI runs format, lint, typecheck, repository/workflow tooling, cospec validation,
managed-file checks and documentation as separate Ubuntu jobs. Native coverage
runs the same way on Linux x86_64, macOS arm64 and Windows x86_64: per-test
partitions in parallel (eight on Ubuntu and Windows, four on macOS, matching
`PARTITIONS` in `packages/kuru-delivery/src/coverage.rs`) and one merge job per
OS on Ubuntu. One Rust orchestrator, `kuru-delivery coverage shard` and
`coverage merge`, sequences both through the `coverage:shard` and
`coverage:merge` tasks. Every partition compiles the same full
workspace/all-target/all-feature instrumented inventory. Its task-private
runner, which Cargo invokes for each test executable with the package cwd and
runtime environment, records the executable's `--list` output and runs only the
tests that a deterministic hash of the artifact identity and test name assigns
to that partition, by explicit `--exact` names. Name lists are split into
several invocations below the Windows command-line limit, and each invocation
must announce exactly as many tests as it selected. Assignment balances test
counts, not durations, and keeps a test in the same partition across commits.

Each partition exports LCOV against its own instrumented executables, with no
threshold, and records a receipt: source commit, tree, Cargo.lock, toolchain and
coverage-tool identity, target, test-profile environment, the artifact
inventory hash, and each executable's listed and assigned test names (stored in
its partition plan and hashed in the receipt). Raw profiles stay on the runner;
the receipt keeps their count, size and manifest digest. The merge rebuilds
nothing. Its fail-closed check is N independent shard builds agreeing, rather
than a separate rebuild: it requires a receipt from every partition, identical
source, toolchain, profile and inventory identity across them, a target
consistent with the OS, and per-executable assignments that are pairwise
disjoint and whose union equals the recorded `--list` output. Any mismatch or
missing receipt fails before any report exists, so there is never a partial
LCOV.

The merged per-OS gate does not use the metric of `--fail-under-lines 90`.
The merge unions the partitions' line records, which must cover the same files
and lines, and requires 91% of unique instrumented source lines (the union of
the partitions' `DA` records) once per OS. cargo-llvm-cov's summary, which
`mise run coverage` enforces locally with `--fail-under-lines 90`, counts each
function-instantiation group's lines separately and cannot be recombined from
partition LCOV. On the same tests it reads 0.67 to 0.75 points lower than the
unique-line figure: on `main`'s green run of the previous topology it was
93.88% against 94.62% (ubuntu-latest), 93.88% against 94.63% (macos-latest)
and 92.57% against 93.24% (windows-latest), and locally 94.33% against 95.01%.
The 91% threshold is an interim margin over that empirical per-OS difference,
not an equivalence: at the largest measured difference it corresponds to about
90.25% by the summary metric, so the CI bar stays at or above the local 90%
gate. The difference is measured, not bounded, and a change that shifts it can
move the effective bar. The durable follow-on is for each partition to export
per-instantiation mapped and covered line sets that the merge unions and checks
against llvm-cov's own totals, failing closed on any mismatch, at which point
the merged gate can return to 90% of the summary metric (see the
`coverage-partitions` design). Because each partition exports
its own LCOV, merging on Ubuntu needs no macOS or Windows runner and no
instrumented objects.

The partition runner stops test executables at a deadline derived from the job's
`timeout-minutes`, less a fixed evidence reserve, and checks it before every
invocation. Compilation is not under that deadline: only the hosted job limit
bounds it, without evidence. A test executable still running at the deadline is
terminated through its owned Job on Windows or its owned process group on Unix,
and the partition fails with a
`<prefix>-coverage-diagnostics-<os>-partition-<k>-attempt-<n>` artifact holding
each executable's output log, a stall report naming the tests libtest reported
as unfinished, the error in `failure.txt`, and the partition's manifests and
runner ledger; any other partition failure uploads the same diagnostics. Only a
successful partition uploads its
`<prefix>-coverage-<os>-partition-<k>-attempt-<n>` receipt artifact, whose root
holds one `attempt-<n>` directory; the diagnostics name cannot match the merge's
download pattern. Rerunning only the failed jobs is enough. The merge refuses
unless every partition job succeeded, then takes each partition's latest
uploaded (successful) attempt for its own OS from the same run and validates it
exactly; an invalid latest attempt is never replaced by an older one. It accepts
both download layouts: one artifact extracted directly into the inputs
directory, or several in directories named after their artifacts. It uploads
`<prefix>-coverage-<os>-attempt-<n>` with the merged LCOV and
`merge-summary.json`.

Each partition also writes a job ledger with phase timestamps, per-executable
start and finish times, invocation and raw-profile counts, the helper and
dependency-seed cache results, and how many dependency and workspace units
Cargo rebuilt. The merge prints these per partition, so a cold or evicted cache
is visible as a slower run. Ledger contents never fail a run.

Partition counts follow the critical path and hosted concurrency limits, not a
fixed ratio. Four macOS partitions and the macOS installation job exactly fill
the account's five concurrent macOS jobs, so nothing in one run queues behind
them; a fifth partition would queue the installation job. Windows is the
slowest OS and sets the run's floor. Ubuntu is not on the critical path, so it
stays at eight and leaves concurrency for other runs. Change a count in
`PARTITIONS` and the workflow matrix together, and only after measuring slack.

On every OS, one installation job runs beside coverage after independently
preparing its locked inputs: it installs the release build offline, verifies the
installed offline runtime and then runs
`mise run //packages/kuru-delivery:test:previous-release-update`, in which the
previous published release's own updater installs the installed executable
under the job's temporary `kuru-bin` directory. It never reads Cargo's target
directory: on Windows, the offline build-input check between installation and
this step has already relinked it with all features. That step needs outbound HTTPS and receives the workflow's read-only
`GITHUB_TOKEN`, used only to list releases. It runs on pull requests, merge
groups and `main` pushes; a branch older than the latest published release fails
it and must be rebased. Linux Clippy does not analyze
platform-specific conditional code; the native suites compile and test those
branches. Linux arm64 additionally builds and packages the native executable and verifies
the packaged offline runtime; that job restores and saves its own per-target
Cargo dependency cache. Its complete real-memory suite runs separately through
three uninstrumented partitions of the same orchestrator
(`//packages/kuru-delivery:test:partition` with `KURU_COVERAGE_PACKAGES=kuru-memory`)
and an Ubuntu merge that applies the same receipt agreement, disjointness and
completeness checks without LCOV or a threshold, and requires every receipt's
package scope to be exactly `kuru-memory`. The arm64 job remains a
supported-platform proof of that whole suite; nothing is subset. The Windows
offline build-input check (`bundle:verify-native-build`) has no test inventory
to partition and stays in the Windows installation job.
Windows primitives retain a separate native coverage job for early feedback. The
required `ci-gate` accepts only success from every branch of this graph.

Ubuntu's partition steps disable Rust test-profile debug information so the
instrumented Kuru executable remains a valid input to the same production
release-archive bound exercised by the packaged-runtime fixture. The value is
part of the test-profile identity and artifact inventory on which every
partition's receipt must agree. Coverage maps, the full test graph, and the
merged line threshold remain enabled. Panic text is retained, but Ubuntu coverage
backtraces may omit source file and line details; use a focused local run or
another native job when those details are needed.

CI installs only each job's tools before task activation, disables automatic
installation of unrelated root tools, and uses `MISE_NO_HOOKS=1` because validation jobs do not create Git
commits. Local Git hooks and maintainer setup retain hk.

On Windows, use the x86-64 MSVC Rust target with Visual Studio C++ Build Tools
and the Windows SDK. The app-owned release task passes an explicit target and
links the C runtime statically; host build tools and procedural macros keep
their normal host configuration. Shipping PE import checks inspect both Kuru
and its embedded Dolt executable for external runtime DLL requirements.

Repository text uses LF line endings on every platform, enforced by
`.gitattributes`. This keeps shell scripts and cospec's generated-file checks
consistent even when Git is configured with `core.autocrlf=true`.

`kuru-platform` owns checked filesystem operations and Windows process/IPC
mechanics; `kuru-archive` owns bounded ZIP decoding. Consumers keep their own
payload policies and use those shared primitives. The PowerShell bootstrap
lives in delivery support and uses stock .NET facilities for its small native
bridge before any downloaded application can be trusted.

## Commands

| Command | What it checks or runs |
| --- | --- |
| `mise run build` | Locked debug workspace build |
| `mise run build:release` | Optimized release build |
| `mise run run -- --provider demo` | Interactive offline harness |
| `mise run format:fix` | Rust, TOML and documentation formatting |
| `mise run format:check` | Rust, TOML and documentation formatting |
| `mise run lint` | All-target Clippy with warnings as errors |
| `mise run typecheck` | Rust compilation checks for all targets/features on the host |
| `mise run test` | Workspace behavioral and protocol tests |
| `mise run coverage` | Run the behavioral suite under LLVM instrumentation, minimum 90% workspace line coverage by cargo-llvm-cov's summary (`--fail-under-lines 90`) |
| `mise run test:install` | Native archive tests and, on macOS/Linux, real Bash bootstrap tests |
| `mise run //packages/kuru-delivery:test` | Delivery contracts, including native PowerShell bootstrap/update fixtures on Windows |
| `mise run //packages/kuru-delivery:coverage:shard` | One fail-closed CI coverage partition, configured by `KURU_COVERAGE_*` ([by hand](#running-a-coverage-partition-by-hand)) |
| `mise run //packages/kuru-delivery:coverage:merge` | Require agreeing receipts from every partition of one OS and, when instrumented, enforce 91% of unique instrumented lines in its merged report |
| `mise run //packages/kuru-delivery:test:partition` | One uninstrumented checked partition of the `KURU_COVERAGE_PACKAGES` test suite (CI arm64 memory) |
| `mise run //apps/kuru-tui:test:embedded-runtime` | Package, install, update and reopen actual Kuru with cold offline memory |
| `mise run //packages/kuru-delivery:test:previous-release-update` | [Previous published release's updater](release.md#previous-release-update-acceptance) installs `KURU_UPDATE_CANDIDATE_BINARY`; optional `GITHUB_TOKEN` |
| `mise run lint:tooling` | Shell, GitHub Actions and metadata validation |
| `mise run docs:dev` | Local VitePress server |
| `mise run docs:check` | Docs formatting/lint, production build, local links, anchors and public content boundary |
| `mise run release:version` | Conventional-commit version calculation without publication |
| `mise run cospec:validate` | Strict validation of changes and durable specs |
| `mise run cospec:managed:check` | Generated cospec-file drift |
| `mise run check` | Optional local aggregate of independently schedulable quality checks |

Choose individual commands for focused work. To request several independent
checks together, use mise's task separator, for example
`mise run format:code ::: lint:rust ::: typecheck`. `format:code` and `lint:rust`
let CI and hooks keep documentation work in the app's single `docs:check` task
graph, whose formatter, linter and build share one dependency installation.
Task dependencies can overlap;
Cargo still protects shared build artifacts with its own locks. A full coverage
run already executes the behavioral tests, so CI does not first run a duplicate
ordinary suite; hk runs neither before a push. Keep only one coverage writer
active per target directory, and preserve the instrumented child fixtures.

Coverage prepares the verified engine archives and uses the supervisor from its
single instrumented workspace build. Its fixtures initialize the engine cache
when needed. Ordinary package tests still prepare a private supervisor snapshot
before they run, so concurrent Cargo builds cannot replace their executable.
Coverage explicitly clears that snapshot opt-in and does not compile an unused
ordinary supervisor first.

`MemoryStore::temporary()` copies a pre-migrated template of one cleanly closed
cold open, then performs the ordinary existing-store open, so each test still
owns its directory, supervisor and Dolt process. Templates live under
`target/<profile>/kuru-test-templates`, one per fingerprint of the supervisor,
schema, engine and schema sources, created and validated under a file lock.
Copies share the template's instance identity, credentials and migration
receipts; tests of lifecycle, migration, import or identity use
`MemoryStore::temporary_cold()`. Old fingerprints are not pruned; `cargo clean`
removes them.

Ordinary application opens start or attach to the internal per-project memory
service from the same Kuru executable. The service owns the prepared Dolt child
and may remain alive for its 30-second idle grace after the last client exits;
it is not an installed system daemon. CLI and PTY fixtures that use a temporary
project must explicitly retire that idle service before removing their fixture
directory. The application still holds the project conversation-driver lease,
so this service boundary does not make simultaneous conversation tests valid.

Development and test builds optimize only the pinned SHA-2 0.11.0 dependency to
keep repeated full-executable update verification responsive. Cargo requires
this version-specific profile override in the workspace root. Workspace code,
debug assertions, coverage instrumentation, CPU dispatch and release profiles
retain their existing settings; every update verification and deadline remains
in effect. The coverage tool instruments workspace crates by default, so this
dependency retains the same instrumentation selection as other external crates.
Update the profile selector alongside any SHA-2 version change.

Coverage includes the application and all packages. Do not exclude hard-to-test
runtime paths or add tautological assertions to inflate the score. Favor tests
that observe peer routing, context isolation, persistence, bounded failure,
protocol payloads and real CLI output. Live authenticated-provider checks are
separate from deterministic fixture tests and must be reported accurately.

## Running a coverage partition by hand

`mise run coverage` remains the local workspace gate. To reproduce one CI
partition, run the orchestrator from a clean checkout of a committed revision
(it refuses modified tracked files and a source other than `HEAD`) with a
throwaway target, evidence and diagnostics directory that do not yet exist:

```sh
scratch=$(mktemp -d)
KURU_COVERAGE_OS=local \
KURU_COVERAGE_SOURCE=$(git rev-parse HEAD) \
KURU_COVERAGE_ATTEMPT=1 \
KURU_COVERAGE_PARTITION=1 \
KURU_COVERAGE_PARTITIONS=4 \
KURU_COVERAGE_TARGET="$scratch/target" \
KURU_COVERAGE_OUTPUT="$scratch/evidence-1" \
KURU_COVERAGE_DIAGNOSTICS="$scratch/diagnostics-1" \
KURU_COVERAGE_JOB_STARTED=$(date +%s) \
KURU_COVERAGE_JOB_MINUTES=45 \
  mise run //packages/kuru-delivery:coverage:shard
```

Any count works locally; CI uses the counts in `PARTITIONS`. The instrumented
build stays in `KURU_COVERAGE_TARGET`, separate from `target/`, so it never
disturbs ordinary builds or the shared build cache; delete the scratch directory
afterwards. On failure, `diagnostics-1/failure.txt` and any stall reports explain
the stop. To exercise the merge, run every index from 1 to the count, each with
its own fresh target, evidence and diagnostics directory, copy each partition's
evidence (its `attempt-1` directory) into a directory named like the CI
artifact, `$scratch/inputs/ci-coverage-local-partition-<k>-attempt-1/`, and run
`mise run //packages/kuru-delivery:coverage:merge` with the same `OS`, `SOURCE`,
`ATTEMPT` and `PARTITIONS`, `KURU_COVERAGE_MODE=instrumented`,
`KURU_COVERAGE_INPUTS="$scratch/inputs"` and a not-yet-existing
`KURU_COVERAGE_REPORT` path whose parent exists. Omitting or altering one
partition's evidence must fail the merge without writing a report.

The uninstrumented mode takes the same inputs plus `KURU_COVERAGE_PACKAGES`, for
example `kuru-memory`, and runs through
`mise run //packages/kuru-delivery:test:partition`; merge its evidence with
`KURU_COVERAGE_MODE=uninstrumented` and the same `KURU_COVERAGE_PACKAGES`, which
the merge requires every receipt's scope to equal. An optional `KURU_COVERAGE_SEED` names a
dependency seed directory to import (below); leave it unset locally.

## Shared build cache

Root mise routes Cargo through [mr-boxington](https://mr-boxington.jdx.dev)
(`mbx`), pinned beside Rust, whenever it runs Cargo for `mise run`, `mise exec`,
activated shells or mise shims. It needs mise 2026.9.4 or later, the root
`min_version`. mbx stores compiled outputs in one content-addressed cache and
restores matching compilations into each checkout's own `target/`, so a new
worktree mostly restores its dependencies instead of recompiling them while
concurrent worktrees keep separate Cargo locks. Calling rustup's `cargo`
directly bypasses the wrapper; use `mise exec -- cargo` from editors and agents.
Do not run `mbx setup`, which writes machine-wide Cargo and editor
configuration.

Root mise sets `MBX_TARGET_VIEWS=0`. mbx would otherwise replace `target/` with
a symlink into its cache root, and the prepared engine inputs under
`target/kuru-bundles` and the private supervisor snapshots refuse symlinked
paths. Keep each checkout's target directory local to it: remove any per-user
`CARGO_TARGET_DIR` override, such as one in an untracked `mise.local.toml`.
Worktrees nested below a checkout inherit its local mise configuration, and a
shared target directory serializes their builds on one Cargo lock. The
checked-in `.mbx.toml` holds only scheduler policy. Cache location, disk budgets
and remote caches belong to each maintainer's own mbx configuration.

Set `KURU_MBX=0` in the process environment to build with plain Cargo. mise
reads it while resolving tools, before any mise `[env]` applies, so setting it
in `mise.local.toml` has no effect. CI workflows, the source installers and
`kuru update --source` set it, so published and user-built executables never
depend on a maintainer cache. Instrumented coverage never reads or writes the
cache: cargo-llvm-cov supplies its own `RUSTC_WRAPPER`, which mbx defers to.

mbx restores outputs by copy-on-write clone on APFS, Btrfs, XFS with reflink,
and ReFS. On filesystems without cloning, such as ext4, it hard-links the
cache's object into `target/` and makes that object read-only first; NTFS
falls back to copies. A hard-linked output therefore has several links and
cannot be written in place. That matches the existing rule to treat Cargo
outputs as read-only inputs whose other links must not be modified, and mbx
unlinks such an output before recompiling it. Plain Cargo, for example after
switching to `KURU_MBX=0` in the same checkout, can report that a restored
output is not writeable; run `mbx clean` or remove `target/` first, or set
`restore_hardlink = false` in your own mbx configuration to restore copies. Local
checks covered only APFS clones; the hard-link path is described by the mbx
documentation and has not been exercised against Kuru's packaging or snapshot
tests.

## Bundled engine build inputs

Every Kuru executable contains its target's pinned full-Dolt archive and license
notices. Installed applications extract this engine locally, including on a
first offline launch. The archive is a build input; memory provisioning never
downloads it at runtime.

`packages/kuru-memory/support/dolt-assets.json` owns the exact version, target,
provenance, sizes and digests. Each asset declares `"provenance": "upstream"`,
with Dolt's release URL, or `"provenance": "built"`, for a target that upstream
never publishes (see [source-built engine inputs](#source-built-engine-inputs)). The memory package's `bundle:prepare` task invokes
the independent Rust delivery helper to download and verify that input. Ordinary
mise build, run, test and check tasks prepare their inputs through dependencies.
To prepare explicitly:

```sh
mise run //packages/kuru-memory:bundle:prepare
mise run //apps/kuru-tui:build:release
```

Preparation can make up to three attempts for the same pinned archive after
HTTP 500, 502, 503 or 504, a timed-out or failed connection, or an interrupted
accepted response body. Each attempt has a 15-second connection and 30-second
read-idle bound, with five- and fifteen-second delays inside the same 120-second
total download deadline; steady progress never extends that total. Error
responses with `Retry-After`, permanent HTTP, size and checksum failures, and
local I/O failures remain fatal. Recovery restarts the immutable GET with a
fresh private stage digest and never publishes partially verified bytes.

The default cache is `target/kuru-bundles` at the workspace root. Each archive is
named `<archive_sha256>.archive`. `KURU_DOLT_BUNDLE_DIR` selects another absolute
directory for both preparation and compilation. Valid files are reverified and
reused; corrupt or unsafe entries fail without replacement. This build cache is
separate from the installed application's extracted `memory.cache_dir`.

Every cached native CI job (coverage partitions and merges, installation and
the Linux arm64 native build and memory partitions) selects a bundle directory
under `${{ runner.temp }}` for all its preparation and build steps. Each OS's
partitions share one Cargo cache key for the delivery helper and registry that
only partition 1 saves. Their instrumented (or, for arm64 memory,
uninstrumented) target directories live at one fixed path in
`${{ runner.temp }}` and are created fresh by the orchestrator; a target is
never restored from a cache. Instead, a separate dependency seed per OS and mode
is restored under `${{ runner.temp }}`, and the orchestrator imports from it,
after creating the fresh target and before the first build, only
allow-listed outputs of non-workspace dependencies. It never imports workspace
crate artifacts, raw profiles, shard state, supervisor snapshots, links or any
other entry, and Cargo's own fingerprints rebuild anything stale. On `main`,
partition 1 exports the seed with the same allow-list after its receipt is
written, and only that export is saved. The seed does not enter any receipt:
inventories record workspace artifacts only. An evicted or missing seed imports
nothing, and the partition is only slower. A seed entry that cannot be read or
moved is counted as an `io` refusal in the job ledger, any partial copy is
removed, and Cargo rebuilds it. An absent, evicted, unreadable or malformed
seed entry never fails the partition, and Cargo rebuilds anything its
fingerprints judge stale; a seeded entry whose bytes are corrupt fails the
build like any corrupt artifact would, without a receipt. Private bundle directories must be
created by the current runner; restoring them inside a Cargo target archive can
change their permissions. Keep them outside shared build-output caches and
retain the private directory checks when configuring native test runners.

Choose a supported target explicitly when preparing or building for it:

```sh
mise run //packages/kuru-memory:bundle:prepare -- --target x86_64-unknown-linux-gnu
mise run //apps/kuru-tui:build:release -- --target x86_64-unknown-linux-gnu
```

The default target is the host; `CARGO_BUILD_TARGET` also selects the consumer
target. Preparation runs its helper on the build host and never executes the
selected archive. Cross-compilation still requires the appropriate Rust target
and platform linker. Unsupported targets fail without substituting a host archive.

For an offline source build, obtain the matching pinned archive from the manifest
and import it into a writable build-input cache:

```sh
KURU_DOLT_BUNDLE_DIR=/absolute/path/to/build-inputs \
  mise run //packages/kuru-memory:bundle:prepare -- \
  --target x86_64-unknown-linux-gnu \
  --archive /absolute/path/to/dolt-linux-amd64.tar.gz --offline

KURU_DOLT_BUNDLE_DIR=/absolute/path/to/build-inputs \
KURU_DOLT_BUNDLE_OFFLINE=true \
CARGO_NET_OFFLINE=true \
  mise run //apps/kuru-tui:build:release -- --target x86_64-unknown-linux-gnu
```

Local imports receive the same size and checksum validation as downloads. The
build-only `KURU_DOLT_BUNDLE_ARCHIVE` variable also supplies a local archive;
`KURU_DOLT_BUNDLE_OFFLINE=true` refuses missing prepared inputs without downloading.
These settings govern engine preparation, so prepare the pinned Rust toolchain
and Cargo dependencies separately before going offline.

Cargo's build script selects by `TARGET`, verifies local bytes again and copies
those verified bytes into its output. It does not download or execute an engine.
Direct Cargo builds therefore require prior preparation; missing or corrupt
inputs fail with the matching mise command. Use
`bash scripts/install.sh --source` on macOS/Linux or
`& .\scripts\install.ps1 -Source` in Windows PowerShell for source installation.
These entrypoints prepare Rust and the bundled input through mise, then install
the complete executable. See
[installation](install.md#build-from-source) for requirements and destinations.

For an offline Windows build, import the manifest's matching ZIP, then build:

```powershell
$env:KURU_DOLT_BUNDLE_DIR = 'C:\BuildInputs\kuru'
mise run //packages/kuru-memory:bundle:prepare -- --target x86_64-pc-windows-msvc --archive C:\Downloads\dolt-windows-amd64.zip --offline
$env:KURU_DOLT_BUNDLE_OFFLINE = 'true'
$env:CARGO_NET_OFFLINE = 'true'
mise run //apps/kuru-tui:build:release -- --target x86_64-pc-windows-msvc
```

The shipping binary is under `target/x86_64-pc-windows-msvc/release/kuru.exe`
unless `CARGO_TARGET_DIR` selects another target directory. For its native import
check, set `KURU_EMBEDDED_TEST_BINARY` to that absolute path and run
`mise run //apps/kuru-tui:verify:windows-imports`. This maintainer check uses MSVC
tools; the installed application does not.

CI's source-install smoke disables both Cargo network access and missing-bundle
downloads after preparing the dependencies. On Windows, the memory-owned
`mise run //packages/kuru-memory:bundle:verify-native-build` task also invokes
the actual Cargo build with isolated missing and same-size corrupt mirrors,
requires the specific build-script rejection, then restores a valid offline
build. Run it after source installation with `KURU_EMBEDDED_TEST_BINARY` pointing
to the installed copy outside Cargo's output directory. It verifies that the
installed executable and original prepared archive retain their hashes.

### Source-built engine inputs

Upstream Dolt publishes no archive for some targets, currently
`aarch64-pc-windows-msvc`, and cannot build them without cgo. A `built` manifest
asset therefore pins everything its archive is made from: the Dolt Go module
version and its `go.sum` `h1:` hash, the ICU source tarball's size and SHA-256,
the Go and llvm-mingw toolchain versions, the recipe name and the single build
host. Preparing, compiling or provisioning any other target never fetches,
builds or reads a built asset's inputs. Runtime provisioning never builds or
downloads an engine.

The build host is **linux-x64 only**. llvm-mingw's target runtimes embed paths
from the package that compiled them, so the same recipe on another host produces
different bytes. Go and llvm-mingw are pinned as task-scoped tools in
`packages/kuru-memory/mise.toml` and locked, for linux-x64 only, by
`packages/kuru-memory/mise.lock`. On linux-x64:

```sh
mise run //packages/kuru-memory:setup:build-tools
mise run //packages/kuru-memory:bundle:build -- \
  --target aarch64-pc-windows-msvc --print-pins \
  --work-dir /absolute/fresh/work --output /absolute/private/out
```

The delivery helper's `bundle build` rebuilds its child environment from `PATH`
and `HOME` only, so inherited compiler, `CGO_*` and `GOFLAGS` settings cannot
change the bytes. It fetches the Dolt module through the Go module proxy and
checksum database and requires the pinned `h1:` hash. It streams ICU through the
same bounded, digest-verified client as `bundle:prepare`, builds static ICU with
stub data, and cross-compiles Dolt with cgo. It then checks that the PE machine
is ARM64 and that only operating-system DLLs are imported. It writes
`<stem>.zip` and `pins.json` to the output directory. The work directory must
not exist; each run starts from empty module and build caches.

The archive holds `LICENSES` (Dolt's Go dependency notices, byte-identical to
the upstream archives) and one file per declared third-party notice beside it:
ICU, the LLVM runtimes and the mingw-w64 runtime. Each notice is read from the
pinned ICU tarball or the locked toolchain and pinned by size and SHA-256. Built
assets must declare notices; upstream assets must not.

Built pins land in two rounds. Until the linux-x64 job has run, the built asset's
archive and executable digests, and any notice digest that has not been observed,
are the literal `"unpinned"` with `null` sizes. Such an asset can be built with
`--print-pins` but can never be prepared or embedded; `bundle:prepare` and Cargo
fail with the build instruction. The `Bundle build` workflow
(`.github/workflows/bundle-build.yml`) builds twice on `ubuntu-latest` in fresh
private directories and fails unless the two archives are byte-identical. It
prints and uploads the archive, its SHA-256 and the observed pins. Commit those
pins; from then on every build verifies against them and any drift fails with
both digests. A pinned built archive is never downloaded: import the CI artifact
or a local linux-x64 build with `bundle:prepare -- --target
aarch64-pc-windows-msvc --archive <file>`.

`KURU_BUNDLE_BUILD_HOST_OVERRIDE=1` lets `bundle build` run on another host for
local iteration on the recipe. Its output is **not authoritative**: it requires
`--print-pins`, never verifies or replaces committed pins, and is labelled as an
override. Put the matching Go and llvm-mingw `bin` directories on `PATH` yourself;
the mise task pins only the linux-x64 toolchain package.

## Change workflow

```sh
mise run cospec -- new feat example-change
mise run cospec -- instructions proposal --change example-change
# Author the indicated artifacts and acceptance ledger.
mise run cospec -- validate example-change --strict
mise run cospec -- apply example-change
# Implement, test, and record actual evidence with the full local gate.
mise run format:code ::: lint:rust ::: typecheck ::: coverage ::: lint:tooling ::: cospec:validate ::: cospec:managed:check ::: docs:check
mise run cospec -- archive example-change
```

Use the change type that matches the intended conventional commit. The apply
gate must succeed before implementation. Archive is performed through cospec
once tasks and evidence are complete, before the final change commit. Generated
schemas and harness instructions are updated by cospec, not edited manually.
After archival, replace any generated purpose placeholders in new durable specs
with their capability purpose and rerun `mise run cospec:validate`.

hk validates format/tooling/specs before commits, separate concurrent static
format, lint, typecheck, tooling, cospec, cospec-managed and docs steps before
pushes, and conventional commit titles. Behavioral tests and coverage with its
per-OS line gate (above) run in CI, not in hooks; run them locally when a
change needs them. Hooks are installed by mise's postinstall and `mise run setup`. Fix failed
checks instead of bypassing hooks.

Git hooks export repository-selection variables, so a subprocess working directory
alone does not isolate another checkout. Delivery's rooted command constructor
clears that inherited context for Git and tools that invoke Git, then applies
intentional overrides such as the temporary release index. Keep new repository
subprocesses on that path and exercise foreign hook environments through child
processes with temporary repositories. See [Git's hook documentation](https://git-scm.com/docs/githooks).

## Memory service protocol

Each typed memory service operation (`ServiceCall`, `ViewOperation` and
`LedgerOperation` in `packages/kuru-memory/src/service/rpc.rs`) has one entry in
its exhaustive `contract()` match. The entry states whether a lost reply may
hide a write, which durable receipt proves the outcome, and the reply budget.
A new variant does not compile until it has an entry. A write must also carry a
receipt; idle retirement is the only exception. Add the variant's sample and
classification row to `service/rpc/contract_tests.rs`.

`service/rpc/protocol-surface.txt` pins the wire surface to `PROTOCOL_MAJOR`
and `PROTOCOL_MINOR` in `service.rs`. It records one sampled shape per variant
for requests and for the nested wire enums they carry: content blocks, turn
transitions, transcript positions, usage proofs, price terms and export cursor
phases. Every optional field in a sample is populated. Response and
unit-valued enums are pinned only by their variant names, and a
free-form JSON value shows only its sampled shape. The handshake hello is not
pinned. Shared `kuru-core` types that
travel on the wire are included. When the pin fails, bump `PROTOCOL_MINOR`.
Older owners then refuse the newer client at the handshake, which is clearer
than a decode failure. Regenerate the fixture with
`KURU_BLESS_PROTOCOL_PIN=1 mise run //packages/kuru-memory:test -- protocol_surface`
and review its diff. Regeneration writes the fixture only when none exists or
when the protocol version is strictly greater than the recorded one.

Residual risks: the fixture is a golden file, so a hand edit, or deleting it and
regenerating, can still record a change at an unchanged version. Review must
reject any fixture diff that comes without a `PROTOCOL_MINOR` bump. Branches
that bump concurrently conflict on the fixture and the constant; the later one
rebases, takes the next minor and regenerates.

## Coding assistants

[AGENTS.md](../AGENTS.md) is the canonical repository instruction source.
`CLAUDE.md` imports it using `@AGENTS.md`, so instructions are maintained once.
Cospec generates the same change workflow for the supported assistants:

| Assistant | Generated workflows |
| --- | --- |
| Claude Code | `.claude/commands/cospec/` and `.claude/skills/cospec-*/SKILL.md` |
| Codex | `.agents/skills/cospec-*/SKILL.md` and `.codex/rules/cospec.rules` |
| OpenCode | `.opencode/commands/cospec-*.md` and `.opencode/skills/cospec-*/SKILL.md` |

To regenerate these integrations while preserving Kuru's existing mise/hk gate:

```sh
mise run cospec -- init --harness claude,codex,opencode --no-gate --yes
mise run cospec:managed:check
```

Here `--no-gate` skips gate scaffolding; the existing hooks and checks remain
configured. Later `mise run cospec -- update` detects the generated harnesses
and refreshes their files from cospec's managed sources. Do not hand-edit them.
Restart Claude Code, start a new Codex session, or reload the OpenCode project
after adding workflows. Codex uses `$cospec-<skill>`; Claude Code uses
`/cospec:<command>` and OpenCode uses `/cospec-<command>`.

Repository permission/approval rules for these assistants are separate from
the generated workflow files above and are authored directly, not by cospec:
`.claude/settings.json` (project permission `allow`/`deny` rules; not listed
in `openspec/.cospec-manifest.json`, so `mise run cospec:managed:check` never
touches it) and `.codex/rules/agent-safe-actions.rules` (a hand-authored
sibling of the cospec-managed `.codex/rules/cospec.rules`; Codex is expected
to load every `*.rules` file it finds under `.codex/rules/`, per the CLI's
own `ignore-rules`/"loaded rules from" behavior — confirm in a live Codex
session, e.g. the "loaded N .rules files in ..." startup log line, since
this was verified with `codex execpolicy check --rules ...` against
explicit file paths rather than through Codex's own project-rules
discovery). Both scope the same safe, routine git/gh/mise/cargo actions and
explicitly exclude force pushes to `main`, release/publish automation,
`mise exec`/`mise x` (an arbitrary-command escape hatch that would bypass
every other deny rule), and other actions that still require an explicit
one-time approval. Update both together when the routine command surface
changes, and re-verify deny/forbidden coverage with `codex execpolicy check`
before relying on a new rule — Codex's rule matcher compares whole argv
tokens, not substrings, so a rule scoped to one task name does not cover a
differently-named sibling task.

## Dependency and release updates

Change workspace dependency pins centrally and regenerate Cargo.lock. Change
tool pins with the matching four-platform lock refresh:

```sh
mise lock --platform linux-x64,linux-arm64,macos-arm64,windows-x64
mise -C apps/kuru-docs lock --platform linux-x64,linux-arm64,macos-arm64,windows-x64
mise -C packages/kuru-delivery lock --platform linux-x64,linux-arm64,macos-arm64,windows-x64
```

Mise records available provenance for every platform, but normally verifies
only the current platform's artifact. Before committing an updated Communiqué
lock entry, verify the other supported archives as well. These commands download
and verify artifact bytes; they neither install nor execute foreign binaries:

```sh
MISE_OS=linux MISE_ARCH=x86_64 mise -C packages/kuru-delivery lock github:jdx/communique --platform linux-x64
MISE_OS=linux MISE_ARCH=aarch64 mise -C packages/kuru-delivery lock github:jdx/communique --platform linux-arm64
MISE_OS=macos MISE_ARCH=aarch64 mise -C packages/kuru-delivery lock github:jdx/communique --platform macos-arm64
MISE_OS=windows MISE_ARCH=x86_64 mise -C packages/kuru-delivery lock github:jdx/communique --platform windows-x64
```

Review the resulting `provenance_verified` metadata alongside URLs and checksums.
This also keeps CI installation from creating uncommitted verification metadata.
Run these lock refreshes with the CI-pinned mise version. Releases after
2026.9.4 no longer add `provenance_verified`, so a lock produced by a newer local
mise would be rewritten by CI's installation and fail its lock drift check.
See mise's [lockfile provenance contract](https://mise.jdx.dev/dev-tools/mise-lock.html#provenance-and-security)
and [task tool configuration](https://mise.jdx.dev/tasks/task-configuration.html#tools).

The memory package's source-build toolchains are locked for their only build
host: `mise -C packages/kuru-memory lock --platform linux-x64`. Their tarball URLs
and digests are mirrored in the built manifest asset, and a delivery test
requires the two to agree.

A built asset's `archive_sha256` also depends on the exact `zip` and `flate2`
crate pins that write the archive. A Cargo.lock refresh that moves either crate
must re-pin that archive digest from the linux-x64 `Bundle build` job; its
executable and notice digests are unaffected. The job fails loudly on a stale pin.

Commit the updated lockfile in the same change. CI detects lock drift. Update
user documentation when flags, configuration, role behavior or contracts change.
[Release operations](release.md) describes manual dispatch, automatic versioning
and publication; [installation and updates](install.md) covers using releases.

## Tests without credentials

Memory tests use actual full Dolt. `packages/kuru-memory` owns bundled extraction,
the supervisor fixture and integration checks. The `test-support` feature is
enabled by package test tasks and their dev-dependency edges, so ordinary Cargo
builds do not include it.
Runtime/TUI test tasks depend on those fixtures. `mise run //packages/kuru-memory:prefetch` extracts and verifies
the embedded engine into the shared test cache. Its build dependency prepares
the archive as described above, downloading it only when needed and permitted.
Cold-cache tests
also exercise first offline extraction, so a populated cache is not a runtime
prerequisite. Test helpers use isolated stores, never the user's memory.
The test runner uses two threads and limits
simultaneous temporary servers. Do not replace these fixtures with SQLite or
exclude memory modules from coverage.

Windows runtime activation retries access denied only after checked observations
prove that the verified source directory has not moved and the destination is
absent. Recovery uses the same source handle, private stage and cache lock for
up to two seconds, with 20-millisecond asynchronous waits. Other errors or
uncertain outcomes fail immediately and preserve the stage. Native regressions
exercise a real held descendant, persistent denial and cancellation; host-only
tests cannot establish this Windows behavior.

The demo provider allows offline process smoke tests. Authentication and
provider fixtures use local OAuth and HTTP peers with synthetic credentials,
including callback validation, token refresh and Responses SSE framing. MCP
OAuth fixtures additionally use isolated, alias-derived native secret entries;
they must delete every test-owned generation and never inspect an existing
credential account. Interruption cases cover callback/device cancellation,
accepted-response publication settlement and manifest recovery. MCP and A2A
fixtures exercise their own wire protocols. Use temporary project,
auth and memory directories; never inspect or copy another application's
credential store to construct a fixture.

OpenAI authentication and requests belong to the Rust connector package.
Running or building these providers does not require Codex CLI, app-server,
Node or npm; the latter two remain docs-app tools. `kuru auth` is a redacted
local status check and does not create credentials or project memory. Real
browser/device sign-in requires user participation, and live model discovery
or inference can refresh Kuru's own session. Record those checks separately
from deterministic fixtures and never include tokens or authorization codes
in verification output.

Windows tests use native process Jobs, private pipes and ConPTY. The delivery
fixtures run stock PowerShell and exercise loaded-image replacement and receipt
recovery. The app's mise fixture installs genuine packaged Kuru through the
actual GitHub backend with isolated simulated release metadata, then reopens
offline memory. This local fixture and the later check of published GitHub
assets provide separate evidence. A host build that excludes Windows tests
does not establish their behavior or coverage.
