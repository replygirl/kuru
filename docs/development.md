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
and arm64, macOS arm64 and Windows x86_64 and arm64 binaries, which cover every supported
platform for the full maintainer gate.

CI runs format, lint, Windows-target lint, typecheck, repository/workflow
tooling, cospec validation, managed-file checks and documentation as separate
Ubuntu jobs. Native coverage
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

Each partition exports LCOV and a line export against its own instrumented
executables, with no threshold, and records a receipt: source commit, tree, Cargo.lock, toolchain and
coverage-tool identity, target, test-profile environment, the artifact
inventory hash, and each executable's listed and assigned test names (stored in
its partition plan and hashed in the receipt). The test-profile environment is
the build and test variables that change what Cargo builds or how tests run,
plus the coverage environment from cargo-llvm-cov's `show-env`. Before it is
digested, the partition's target path becomes a fixed token, and so does the
merge-pool size `N` of the `%<N>m` specifier in the profile file name, which
`show-env` sets to the host's available parallelism: hosted runners of one
image can report different CPU counts. Every other value, including a
`%m` without a size, is compared as written. The partition uploads this
neutralised environment as `profile-env.json`, whose digest is the receipt's;
it holds Cargo and test settings, paths, flags and crate names, never
credentials. Raw profiles stay on the runner;
the receipt keeps their count, size and manifest digest. The merge rebuilds
nothing. Its fail-closed check is N independent shard builds agreeing, rather
than a separate rebuild: it requires a receipt from every partition, identical
source, toolchain, profile and inventory identity across them, a target
consistent with the OS, and per-executable assignments that are pairwise
disjoint and whose union equals the recorded `--list` output. The merge
requires each uploaded `profile-env.json` to match its receipt's digest and,
when two partitions' environments differ, names every differing key with both
values. Any mismatch or missing receipt fails before any report exists, so
there is never a partial LCOV.

The per-OS gate is 90% by the metric `mise run coverage` holds to
`--fail-under-lines 90`: cargo-llvm-cov reads `totals.lines` of
`llvm-cov export`, which counts, per source file, each function instantiation
group once (the functions starting at one location, such as a generic's
instantiations or a library built with and without `cfg(test)`), with the most
mapped and the most covered lines of any of its instantiations. Summed profiles
cannot be recombined from per-file LCOV, so each partition also writes
`coverage-lines.json`: every instantiation's source file, name, group location
and mapped and covered lines, derived from its full `llvm-cov export` JSON with
a port of llvm-cov's line statistics (LLVM 22.1.8, the pinned toolchain's
`llvm-tools`). Before its receipt is written the partition runs
cargo-llvm-cov's `--json --summary-only` report over the same profiles and
requires the export to reproduce it exactly, per file and in total; a mismatch
fails the partition, names the first mismatching files and keeps the summary in
its diagnostics. The receipt carries the export's digest and self-checked
totals.

The merge refuses a receipt without that digest or an export that differs from
it or from its totals. It requires every partition to report the same files,
instantiations, group locations and mapped lines, unions each instantiation's
covered lines, and sums each group's maximum per file and in total. That equals
the summary of the summed profiles: the mappings are identical and region
counts are non-negative, so a region's summed count is nonzero exactly when
some partition's is, and a line's count is the maximum of a structurally chosen
set of region counts. A negative counter expression (for example, when
instrumented code is still running on a detached thread while the process
writes its profile at exit, so the written counters are mutually inconsistent;
the counter updates themselves are atomic) would break that equality, since
its sum can cancel across partitions where no self-check sees it. `llvm-cov
export` clamps such a count to `i64::MAX`, which no real count approaches, so
the line export refuses any region with that count, naming the function and
region, and the partition fails and must be rerun. The equality also needs the
partitions' regions to be all counted code regions, which is all rustc emits;
a skipped or zero-length region would make llvm-cov's segment suppression
count-dependent, so the export refuses them, naming the function. The merge
prints each file's figures and the total against the gate. It also unions the partitions' LCOV, which must cover
the same files and lines, into that OS's merged report, and prints its
unique-line percentage for information only: a line shared by several groups
counts once there, so it reads higher than the gate metric. Because each
partition exports its own coverage, merging on Ubuntu needs no macOS or Windows
runner and no instrumented objects.

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
this step leaves the memory build-script fingerprint dirty after its rejected
inputs. Each installation job compiles the release executable once; the Windows
check reuses the source installation's build as its positive control instead of
rebuilding it. That step needs outbound HTTPS and receives the workflow's read-only
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

Windows on Arm (`windows-11-arm`) runs the same partitioned workspace suites as
x64, uninstrumented, as separately named behavioral evidence: `Behavior
partition` 1..8 run `//packages/kuru-delivery:test:partition` over every
workspace package, and `Behavior merge` checks receipt agreement, disjointness
and completeness on Ubuntu without LCOV or a threshold. Their receipts record
`mode: uninstrumented` with null coverage, and they never count toward the 90%
gate, which x64, macOS and Ubuntu keep enforcing. Instrumented Windows on Arm
partitions are held until a pinned Rust toolchain carries the fix for
[rust-lang/rust#150123](https://github.com/rust-lang/rust/issues/150123), whose
`llvm-profdata merge` failure on `aarch64-pc-windows-msvc` the pinned 1.98.1
reproduces; `coverage::PARTITIONS` rejects an instrumented `windows-11-arm` set.
`native-platform` is a two-leg matrix: the x64 leg keeps
`//packages/kuru-platform:coverage` and its 90% gate, and the arm64 leg,
`Native platform behavior (aarch64-pc-windows-msvc)`, runs
`//packages/kuru-platform:test` with no coverage upload. The arm64 partitions
and installation job import the source-built engine from the
`dolt-windows-arm64` job before building (see
[source-built engine inputs](#source-built-engine-inputs)).

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
| `mise run lint:windows` | The same Clippy for `x86_64-pc-windows-msvc`, so `cfg(windows)` code is linted ([details](#lint-configuration-and-windows-target-lint)) |
| `mise run typecheck` | Rust compilation checks for all targets/features on the host |
| `mise run test` | Workspace behavioral and protocol tests |
| `mise run coverage` | Run the behavioral suite under LLVM instrumentation, minimum 90% workspace line coverage |
| `mise run test:install` | Native archive tests and, on macOS/Linux, real Bash bootstrap tests |
| `mise run //packages/kuru-delivery:test` | Delivery contracts, including native PowerShell bootstrap/update fixtures on Windows |
| `mise run //packages/kuru-delivery:coverage:shard` | One fail-closed CI coverage partition, configured by `KURU_COVERAGE_*` ([by hand](#running-a-coverage-partition-by-hand)) |
| `mise run //packages/kuru-delivery:coverage:merge` | Require agreeing receipts from every partition of one OS and, when instrumented, enforce the 90% gate by cargo-llvm-cov's line metric over its partitions' line exports |
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

CI jobs do not fetch what they do not use or what the run has already
verified: each such download is one more outage that can fail a job needing
nothing from it. The repository check in `lint:tooling` (`kuru-delivery repo`)
enforces the checkable part on every workflow. A workflow that runs mise sets
`MISE_EXEC_AUTO_INSTALL: "false"` and `MISE_TASK_RUN_AUTO_INSTALL: "false"` at
workflow level and never overrides them in a job, step or script
([why](#shared-build-cache)). Every `jdx/mise-action` step's `install_args`
names at least one tool, not only options, and no step runs `mise install`,
`mise upgrade` or `mise bootstrap` without tool arguments, whatever mise
options come before or after the subcommand and whatever `sudo`, `env`,
`timeout`, `nice`, `nohup`, `exec` or `time` prefix, with its option values,
precedes it. Every `apt-get` or `apt` fetch, including one inside a `sh -c`
string, names exactly one list with `-o Dir::Etc::sourcelist=/...`, ends its
parts settings with `-o Dir::Etc::sourceparts=/dev/null`, and reads no `-c`
file; no workflow, job or step sets `APT_CONFIG`, and `add-apt-repository`
runs with `-n`, so the runner image's third-party repositories are never
refreshed. Every matrix job that runs a partition task (`coverage:shard` or
`test:partition`) or has a `partition` axis runs with
`KURU_DOLT_BUNDLE_OFFLINE: "true"` and imports the archives that the CI
`bundle-inputs` job fetched and verified once for the run
([import](#bundled-engine-build-inputs)).

The release workflow is held to the same rule; no job is exempt. PR CI cannot
run it, so the delivery package's `release_workflow` tests stand in for its
`notes` and `build-docs` jobs. For each job they derive the tools its steps
need from the workflow text and the mise task graph those steps reach: each
task's own `tools`, the programs its commands run, and its dependencies. They
require every tool to be installed by name in an earlier command, either in
`install_args` or in a named `mise install`. A tool installed by a dependency
inside the same `mise run` does not count, because that run's PATH is fixed
before the dependency installs it. An install counts only for the version it
installs: an explicit `tool@version` must equal the version the task declares
or the configuration pins (with `{{vars.*}}` resolved), and an install without
a version counts as the version configured where it runs. They also reject:

- an install that names no tool;
- an install step with `if` or `continue-on-error`, whose installs then count
  for nothing, and a mise-action input other than `experimental`, `version`
  and `install_args`;
- workflow or job `defaults`, and a step `working-directory` or `shell`;
- a `MISE_` variable in the job's or a step's `env`, and a workflow-level one
  other than `MISE_LOCKED`, `MISE_EXEC_AUTO_INSTALL` and
  `MISE_TASK_RUN_AUTO_INSTALL`;
- a step that writes `GITHUB_ENV` or `GITHUB_PATH`.

The derivation reads Linux `run` commands only and knows the programs those
two jobs reach; a new program fails it until someone records which tool
provides it. These limits remain:

- it does not see what a compiled tool launches beyond its task's declared
  tools;
- it does not read a task's own `env`, `dir` or `shell`, or environment
  variables other than `MISE_` ones that could change a tool's behaviour;
- it treats the root `postinstall` hook (`hk install --mise`) as outside the
  jobs' tool needs, since a failing hook only warns;
- it trusts mise to install the configured version for an install without
  one, and does not model a job-level `if` or `continue-on-error`, which skip
  or tolerate the whole job rather than one install;
- it cannot prove that the release runner's mise behaves as the model assumes;
  only a real release run does.

The check reads workflow text only, so these remain outside it:

- downloads inside the mise tasks a step runs, and settings in a task's own
  `env`;
- the tools a job does select, the single `bundle-inputs` fetch and the one
  engine download of each installation or native build job;
- a launcher that clears the environment, such as `env -i` or `sudo`'s
  default `env_reset`, which drops the workflow-level opt-outs before mise
  runs;
- spellings assembled at run time: a variable name split by quotes and
  written to `$GITHUB_ENV`, a task name split by quotes or passed through a
  variable, an `install_args` expression that evaluates to nothing, and a
  quoted `;` or `|` inside a command;
- whether a named apt list is the one a step needs, and an engine fetched
  with `curl` then imported with `--archive`;
- `mise bootstrap` with a subcommand, such as `bootstrap packages apply`: it
  runs a single part, not the tool phase, but a package part may run a system
  package manager the check does not see.

Coverage prepares the verified engine archives and uses the supervisor from its
single instrumented workspace build. Its fixtures initialize the engine cache
when needed. Ordinary package tests still prepare a private supervisor snapshot
before they run, so concurrent Cargo builds cannot replace their executable.
Coverage explicitly clears that snapshot opt-in and does not compile an unused
ordinary supervisor first.

`MemoryStore::temporary()` copies a pre-migrated test template of one cleanly
closed new store, then performs the ordinary existing-store open, so each test
still owns its directory, supervisor and Dolt process. The test template's
source store is itself created from the production store template below, so
its retained migration branches carry that template's placeholder identity.
Templates live under `target/<profile>/kuru-test-templates`, one per
fingerprint of the supervisor, schema, engine and schema sources, created and
validated under a file lock. Copies share the template's instance identity,
credentials and migration receipts; tests of lifecycle, migration, import or
identity, and tests that pool a retained migration branch, use
`MemoryStore::temporary_cold()`. Old fingerprints are not pruned; `cargo clean`
removes them.

### Shared store template and fixture warm-up

The production store template cache (see [memory](memory.md#new-projects-and-the-store-template))
lives beside the engine in the shared test cache: `KURU_DOLT_CACHE`, or
`kuru-dolt-test-cache` under the system temporary directory, at
`<cache>/<engine version>/templates/<key>/`. It holds a captured `data/` tree
only, keyed by the schema, the engine and the creation statements, not by the
supervisor executable, so an instrumented and an ordinary supervisor build
interchangeable templates. The key also covers the migration publication
record format. Fixtures are at schema 8, whose step records each published
migration branch, so a change that adds a schema step or changes the record
format changes the key. The next `prefetch` or first fixture open then builds
the template once more; nothing has to be removed by hand. It differs from the test template above, which is
a whole closed store keyed by supervisor bytes, sources and scope; the test
template's own source open runs after the production template is warm, so it
copies the production template.

Every fresh fixture open (a writable open of a store that does not exist yet)
now creates its store from the warmed production template: two engine starts,
the copy's adoption start and the active start, in the engine ledger. The
first open for a template key that finds no published template builds it
first, three starts; with warm-up in place a fixture never does, and the guard
below fails one that did. `Creation::Cold` fixtures, a legacy import and a
configured `dolt_binary` take the cold staged build, also two starts: the
stage's one start, which initializes, imports, migrates, validates and marks
the stage ready, and the active start. So does any open that falls back from a
busy or unusable template. Fixture deadlines (`test_support::fixture_deadline`)
budget two starts per fresh open, the most a fixture open can take now that
the guard forbids a build; `test_support::fresh_open_budget_of` budgets each
path (`FreshOpen::Template`, `FirstProject`, `Cold`) for tests that assert
one, and `fresh_open_budget` is the first project's, the longest. Tests that
pause or delay the cold staging job (migration hooks, the stage pool delay)
set `Creation::Cold`, since a template copy runs no migration. Tests of an empty, busy or damaged template set
`OpenOptions::template_root` to a private root, so the shared template other
fixtures copy is never disturbed. A spawned-binary fixture that gives Kuru
its own empty `memory.cache_dir` (a first launch on a fresh machine) has its
first new project build the store template in that cache; the template's
captured database repository belongs to no store, so the fixture root's
depth budget (`TempDir::with_depth_budget`) is the cache directory's own depth
plus `test_support::TEMPLATE_DEPTH`.

The native mise fixture (`windows_mise`, through the delivery package's
`mise_acceptance` support) keeps its cold-cache assertion, then warms its own
engine cache before its first launch, as `prefetch` warms the shared one: it
provisions the engine into it within `test_support::engine_warm_up_bound()`,
then calls `test_support::warm_template_cache` with the installed binary as
the build's supervisor. That launch, run with `KURU_OPEN_MARKERS=1`, then
copies the template with two engine starts instead of also unpacking the
engine and building the template under one client wait; its standard error
must show the opening sentence and never the getting-ready one. A
`test_support::TemplateCacheReceipt` taken after the warm-up, whose
`verify_used` fails on any other key, build or quarantine in the templates
root, and the store's recorded template key
(`test_support::store_template_key` equal to `test_support::template_key()`)
prove the launches used it. Warm such a cache only for a binary built from
the same commit. The packaged `embedded_runtime` fixture stays cold in the
Installation job on every OS: it is the suite's proof that an installed
executable unpacks its own engine and builds its own template in one launch.
Under coverage only (when `LLVM_PROFILE_FILE` is set), it warms each
installation's own cache the same way before that installation's first launch,
since an instrumented build is not a product condition, and checks the
launch's standard error, the receipt and the store's template key as above.
Its `--nocapture` run prints one
`embedded_runtime first launch (cold, …): <ms> ms [<label>]` line per
installation, or `(warm (coverage), …)` under coverage, a measurement with no
threshold.

`test_support::warm_runtime_cache()` warms both halves of the shared cache
once per test process, before any fixture deadline: it provisions the engine,
then checks the published template's structure under its shared key lock, or
takes the exclusive key lock and builds and publishes it, waiting by polling
for another process's build within one fresh-open budget. Only a successful
template warm-up is cached, so a failure fails the fixture that met it, by
name, and the next fixture tries again. A lock-file error is fatal in
warm-up, never treated as "no template". The `prefetch` task builds the
template after provisioning, with its prepared supervisor snapshot, so the
ordinary `test` tasks start warm; coverage does not run `prefetch`, and its
instrumented fixtures warm the template in the instrumented test process
with the instrumented supervisor.

Fixtures obtain their options through `test_support::warmed_open_options()`
or `OpenOptions::warmed()`, both async, and build them before any spawn gate:
the warm-up holds a shared `spawn_gate` guard, and the gate is fair, so a
first warm-up under a caller's own guard could deadlock behind a queued
writer (the lib test `no_spawn_guard_encloses_a_test_cache_warm_up` scans
the sources for one). `test_support::open_options()` alone returns options
marked unwarmed; a writable open of a store that does not exist yet fails at
once with those options in the shared cache, before any engine starts, so
every fixture that could build the template inside its open is found on its
first run. Reopens, read-only opens and options naming another cache are not
refused. `MemoryStore::temporary()` and `temporary_cold()` warm first, before
the test template's lock and the fixture permit. `Creation::Cold` fixtures
are warmed too: they need the engine.

Spawned processes cannot carry that mark, so `test_support::cache_dir()`
warms before it returns the shared cache, on a private thread with its own
runtime (`test_support::warm_blocking()`), and refuses to run inside a Tokio
runtime; a caller inside one awaits `test_support::warmed_cache_dir()`
instead. `test_support::spawn_logged_owner` warms with the async form. In the
application tests each sandbox has a synchronous constructor for plain `fn`
tests and an async `warmed` one for tests inside a runtime.

Test support charges a fixture whose open built or quarantined the shared
template, and the fixture root's teardown fails it, unless the root opted in
with `TempDir::allowing_template_build()`: the template cache's own tests do,
and use private template roots. Warm-up never fails for finding a
quarantined `.rejected-*` directory.

Nothing prunes the shared test cache. It keeps every engine version and
every template key built on the machine, across worktrees; key lock files are
permanent, as are the Windows build-store leases under `templates/lifecycles/`
(one small file per build). Reclaim it by deleting the whole cache directory
while no test runs anywhere on the machine.

Ordinary application opens start or attach to the internal per-project memory
service from the same Kuru executable. The service owns the prepared Dolt child
and shuts down as soon as its last client and accepted work have drained; it is
not an installed system daemon. The command that exits does not wait for that
close, so CLI and PTY fixtures that use a temporary project await the owner's
exit (`test_support::await_owner_release`, below) or its quiescence before
removing their fixture directory. The application still holds the project conversation-driver lease,
so this service boundary does not make simultaneous conversation tests valid.

Dolt panics at close when its data directory disappears before it exits, so a
fixture root from `kuru_memory::test_support::tempdir()` requires, when it
drops, a quiescence record for every memory store beneath it: a directory
holding the Unix `lifecycle.lock`, a directory holding `identity.json` whose
Windows lease `lifecycles/<identity>.lock` exists, or the store a service owner
lock names. A template or an unopened copy has no lease and is not a store. It
never probes a lock: on Unix a lock released in-process can stay held by a
sibling thread's child between `posix_spawn` and `exec`, so a lock's state
cannot tell a live owner from a released one. The records are process-local
and written only on evidence this process observed itself:

- Closing a store records it. Every Dolt supervisor the test process spawns
  stays live in the ledger until the process reaps that supervisor, and the
  reap records the store.
- `test_support::await_managed_quiescence(&options)` retires a managed owner
  that is still running, then waits for the lifecycle lease of the project store, of each of
  its `<hash>.staging-<uuid>` siblings and of each stage preserved under
  `interrupted/` (bounded by the supervisor's reap allowance; a timeout fails
  the test) and records each while its lease is held. Managed fixtures call it
  after their clients close. A fixture that does not know its projects, such as
  the application's `ServiceCleanup`, finds them with
  `test_support::managed_store_scopes(&data)`, which uses the same recognition:
  a fresh open runs its engines under the staging name and renames the store
  only once activation is validated, so a project whose owner in another
  process has not activated it has no store under its plain digest. Never
  enumerate stores by a hand-written name pattern. A fixture that already has
  an outcome releases through `ServiceCleanup::release(outcome)`, which
  attaches a cleanup failure or the guard's verdict to that outcome.
- `test_support::await_store_quiescence(&directory, lifecycle_root)` does the
  same for one store whose engine ran in another process, such as a spawned
  `kuru` executable.

A kuru-memory fixture that serves an in-process `ServiceOwner` on a task must
retire it on every exit path: an early `?`, `bail!` or `ensure!`, the end of
its success path, and an elapsed fixture deadline. It creates its root and
options outside its deadline, starts one `test_support::FixtureDeadline` for
the whole fixture (shared by every stage and loop iteration), and serves the
owner through `FixtureDeadline::serve`. The stage it passes opens the owner,
serves it through the `ServedOwner` it is given and returns the body's
`Result`; the teardown it passes is `ServedOwner::retire` (the maintenance
permit, then the owner's reap, with the bounds of the fixture's success path).
The helper holds the served owner outside the timed future. Within the
deadline it writes a body error to the test's captured output before the
teardown starts, and returns the body's error as the root cause with any
teardown failure attached. When the deadline elapses, the stage and its
clients are dropped, the deadline's own error (`<fixture> exceeded its <budget>
deadline`) is written first, and the same teardown then runs, bounded by its
own permit and reap bounds. Stages that serve no owner run through
`FixtureDeadline::run`. The fixture releases its root with
`TempDir::release(outcome)`: when the teardown could not retire the owner, for
example because an aborted request still holds a client attachment, the root
is kept and the guard's verdict is attached to the fixture's error instead of
replacing it with a panic. A root that is dropped rather than released still
panics on a violation. Guarded roots that other fixtures create inside a
`tokio::time::timeout` future still report an elapsed deadline through the
guard's panic; converting them to `FixtureDeadline` is a recorded follow-on.

The guard's scan reads at most 8 directory levels and 4096 entries beneath its
root. It skips exactly one directory per store: the database repository
`<store>/data/kuru/.dolt` of a store it has recognised from its lease files.
That repository nests past the depth budget in every real store, Kuru writes
no store marker, lock file or identity record inside it, and the engine's own
files there are covered by that store's quiescence record, not by the scan.
Every other directory named `.dolt` is scanned and subject to both budgets:
one outside any store, the repository of a directory no lease recognises (a
template or an unopened copy), and any other `.dolt` beneath a store, such as
`data/.dolt` or `home/root/.dolt`. Whatever the scan cannot read fails the
teardown like an unexplained store: a directory past the depth budget, an
entry past the entry budget, an unreadable directory or entry, and a store
whose identity cannot be taken. A fixture whose root outgrows either budget
must scan a narrower root or name a larger budget at its own call site with
`TempDir::with_depth_budget`, which can only raise the default; the budgets
are never raised globally. Every store carries directories the scan reads
four levels below it: its supervisor runs Dolt with `DOLT_ROOT_PATH`
`<store>/home/root`, where Kuru stages Dolt's global config in
`.dolt/staging` and Dolt creates `.dolt/eventsData`. A fixture whose data
directory lies three or more levels below its root exceeds the default 8
levels at `home/root/.dolt/eventsData`. The two template fixtures, which keep
a template and unopened copies, name an 11-level budget at their call sites.
The packaged install and update acceptance in
`apps/kuru-tui/tests/embedded_runtime.rs`, whose data directories are three
levels down and whose empty engine cache holds a provisioning probe with the
same Dolt root, names a 10-level budget.

A kuru-memory fixture that releases a lock and takes it again at once through a
one-shot acquisition, such as a successor `ServiceOwner::open` after its
predecessor retired or closed, holds the exclusive spawn gate from before the
release until that acquisition returns. The shared spawn guard does not exclude
a sibling test's spawn, and that sibling's child can hold a duplicate of the
just-released lock description until its `exec`, so the successor would see a
busy lock although no owner exists. `ServedOwner::restart` retires, opens and
serves a successor this way, and `spawn_gate::excluding_spawns` covers any other
sequence; both take the caller's shared guard and hand the exclusive guard
back down to it atomically, because a successor served inside the restart
is served inside the restart and must not wait for the gate afterwards.
`ServiceOwner::open` stays one-shot and a fixture never retries it. The gate is
fair, so a restart waits for every running spawner to drop its shared guard,
within the fixture's own deadline, and code under the exclusive guard must not
take a shared one.

The store template's key lock needs no such gate. Every release of it unlocks
explicitly before its handle closes (`creation_template::KeyLock` and
`files::release_lock`), including its release as a template build engine's
reap guard, so a sibling's child holding a duplicate of a released key-lock
description holds no lock: a new project's exclusive try after its shared
inspection, and a quarantine after a verdict, meet a busy lock only when
another holder exists. The tests
`a_released_key_lock_is_free_while_a_duplicate_descriptor_remains` and
`an_inherited_duplicate_of_the_shared_key_lock_does_not_send_the_opener_cold`
hold such a duplicate deliberately.

The project service's start and owner locks follow the same rule:
`ServiceLock::release` and the lock's drop both unlock explicitly before the
handle closes. A `ServiceOwner::open` that fails releases its owner lock after
its store has closed and reaped Dolt, and before the open returns, so a starter
can elect a successor on that lock at once. The test
`a_failed_open_releases_its_owner_lock_despite_a_duplicate_descriptor` holds a
duplicate of the failing owner's lock across its open. A maintenance permit
holds both locks and releases the owner lock before the start lock, the reverse
of their acquisition, so a starter that wins the start lock does not meet a
departing permit's owner lock.

An owner ends itself when its last attachment releases, so a fixture that
needs a running owner chooses its lifetime policy. `ServeKnobs`
(`service.rs`) carries the admission rule, the first-attachment deadline, the
lock recheck interval and the test hooks. `Admission` is `Starter(token)`
(retire only after the attachment presenting that token has attached),
`AnyAttachment` (the mixed-version fallback for an owner started without a
token) or `Never`. `ServedOwner::serve` uses `Never` with no deadline, so an
in-process fixture owner ends only by maintenance retirement, `restart` or lock
loss; `ServedOwner::serve_with(owner, knobs)` and `ServiceOwner::serve_with`
choose another policy, and the retire-on-last-detach path is covered by
tests with a really spawned owner and by the command-line tests. Tests never
sleep for an owner's lifetime; they follow events:

- The serve-loop observer (`ServeKnobs::observer`, an ordered unbounded channel
  of `ServeEvent`) reports `AttachmentAccepted { active }`,
  `AttachmentJoined { remaining }`, `EnteredEmpty { reached }` and
  `LockRechecked`. Events are sent before the loop acts on them, so an early
  retirement always appears in order before any later event. The consumers
  `observed`, `next_event`, `expect_events` and `expect_no_event` live in
  `test_support/served_owner.rs`.
- `ClosePause` (`ClosePoint::BeforeListenerDrop`, `AfterListenerDrop`,
  `AfterEndpointRetire`, `AfterReap`) holds a closing owner at those points, as
  `ReplyPause` holds a reply, to test racing clients, close order and
  maintenance. `DispatchPause` holds the next request of any kind before
  dispatch; unlike `RegisteredPause` it also pauses reads.
- `test_support::await_owner_release(&options)` waits for a spawned owner in
  another process by taking and releasing its Owner lock (a blocking lock in
  `spawn_blocking`, no deadline of its own). The wait cannot be cancelled, so a
  caller bounds it from outside, as the application's `await_owner_exit` does,
  and calls it only while nothing is electing. With the `test-support` feature,
  `KURU_TEST_MEMORY_OWNER_DIAGNOSTIC` names an existing file that a command's
  spawned owner appends its standard error to.
- The `RemoteSession` replacement hook (`installed`, `discarded`, `failed` and a
  pause after connect) observes the replacement connection a cancelled call
  starts; it is `cfg(test)` inside `facade.rs`.

The starter token is a random UUID the starting client passes to the owner it
spawns as an optional tenth service argument and presents in its handshake hello.
Only an attachment presenting it marks the owner as reached by its starter, so
a `kuru memory status` or a second client cannot retire a new owner before its
starter attaches. An owner started without one (a client from before the token,
after an update replaced the executable) treats any authenticated attachment as
reaching it.

While it opens, the owner publishes the open stages it has begun as a small
private record beside its endpoint (`activity.json`, in `service/activity.rs`).
The record is format 2 only (`deny_unknown_fields`, at most 4 KiB) and carries a
SHA-256 value derived from the starter token, never the token. Beside the stages
it carries `progress`, a count that belongs to one open
(`MemoryStore::open_observed` creates it; the open's `ProgressReporter` and
`ServerOptions` carry it). The count advances only at a distinct one-shot point or a completed bounded unit of
work: every stage report, every open-timeline milestone stamped inside the open
(stamp and advance are paired at the site), every 8 MiB copied by extraction or
hashed by warm-cache verification, and every completed migration step. A new
site follows the same rule: never advance on a timer, inside a sleep, in a
retry iteration or while waiting for a lock. Immediately before an open that
has failed closes an engine it started, and only where the open then returns
that failure, the owner marks the record `"failing": true` with a `"reason"`
taken from the error's text, cut at a character boundary to stay within the
record limit; the starter token's spellings, the tag and the store's connection
secrets are replaced in it. An owner whose store opened but which ends before
its starter attached, because its listener, endpoint publication or serve loop
failed, marks the record the same way (with the endpoint's connection secret
replaced too) while it holds verified owner authority, before its store closes.
It awaits that one write, as retirement already awaits the publisher, then
closes the store and retires the record; without verified authority, or when
the write fails, it retires the record before the store closes. Otherwise that
starter would see only the retirement and lose the reason. The publisher writes a stage change or the failing
mark at once and coalesces progress-only changes to at most one write every
250 ms, always writing the latest. The starting client reads only a record
carrying the value for the token it passed, once per readiness poll after that
poll's attach attempt and owner-exit check (`OwnerWatch`). A change of its
stages, count or failing mark restarts the `memory.startup_timeout_secs`
window; a record it read that is gone, or one marked failing, ends the wait at
once (see [configuration](configuration.md)). Gone means that the lookup of the
record's name, or of its directory, found nothing (`files::is_missing_name`). A
record replaced over its name after the client opened it fails the platform's
handle check as unlinked, with no OS error code (Unix), or as delete-pending
(Windows); either way it is not a missing name, that read decides nothing and
the next poll's lookup does. Beyond extending or ending its own
starter's readiness wait, the record grants no authority: election,
attachment, recovery and retirement never read it, and a failed write never
fails or delays the open. The owner always replaces it by publishing a complete
staged record over its name, and retires it (rename, then remove) inside its
close, after the endpoint is retired and before the store closes (after it,
when it marked the record failing as above), and on every error return of its
open. Two hooks, read only by
the owner process under `test`/`test-support` (Windows owners receive them by
explicit forwarding), let tests follow events instead of sleeping:
`KURU_TEST_MEMORY_ACTIVITY_WRITE_FAILURE=1` makes every record write fail, and
`KURU_TEST_MEMORY_OPEN_HOLD_DIR=<dir>` holds the owner's open at stage `X`
while `<dir>/X.hold` exists, once a record holding `X` has been written, until
that file is removed. Tests set them on the command-line child, never on the
runner's own environment (`test_support::{WRITE_FAILURE_ENV,
OPEN_HOLD_DIR_ENV}`), and remove a hold once the sentence for that stage is
visible. `test_support::hold_owner_lock` takes a project's owner lock, so a
child that elects an owner finds it busy and shows the waiting sentence until
`HeldOwnerLock::release`.

The progress-bounded readiness wait is tested against a real child process on
Linux, macOS and Windows through a stand-in owner. With the `test-support`
feature, `KURU_TEST_MEMORY_SERVICE_STAND_IN=<file>` (`SERVICE_STAND_IN_ENV`)
makes `service_entry` skip everything else: the owner takes no lock, opens
nothing and writes nothing, and exits with the decimal status `<file>` holds
once it holds one, polling every 10 ms of real time in its own process under a
120 s bound so an unreleased stand-in leaves no process behind. Tests pass it
only through the task-local owner environment
(`activity::with_owner_environment`), which both platforms' spawns apply, and
spawn the prepared `test-support` snapshot; the test then writes and retires
the owner's record itself from the starter's poll hook, on a paused clock where
the test needs exact windows. Inside the crate, `cfg(test)` seams on the
publisher's `Writes` (a write gate, a write log and a mode that logs without
I/O, since a paused clock cannot be trusted across `spawn_blocking`) check its
write spacing, and `store::failed_open_close` pauses one test's own open
between its failing mark and its engine's close. `ClosePoint::BeforeStoreClose`
pauses an owner just before its store closes, after its record's mark or
retirement, through its serve knobs or, for a failed endpoint publication,
`OwnerHooks::close_pause`; `ServeKnobs::accept_fault` makes the serve loop's
accept fail. The application tests' shared `ServiceCleanup` fixture creates a
private `KURU_TEST_MEMORY_OWNER_DIAGNOSTIC` file in its root, and the
command-line children of the `cli`, `lease`, `preferences`, `terminal`,
`trust`, `unix_shell_turn` and `windows_terminal` binaries and of `server`'s
Unix leg receive it. When the fixture drops, it writes any stderr those owners
left to the test's captured output, which the harness shows only for a failing
test, whether it panicked or returned its error; `ServiceCleanup::release`
attaches the same text to a failed outcome. Both read the file after cleanup
has awaited the owners it can find. With the `test-support` feature an owner
whose open fails writes `memory service owner open failed: <error>` to its
stderr before it retires its activity record, so the file already holds the
reason when the starter sees the record gone; the process's own final error
report follows only after its owner lock is released. A starter run with
`KURU_TEST_MEMORY_STARTUP_STAGES=1` sends its owner's stderr to a private file
and, when its start fails, appends that file (at most 1 MiB) to the owner
diagnostic. Owners that the test process elects itself, through an in-process
open, are not covered: the test runner's own environment is never changed.

With `KURU_OPEN_MARKERS=1` (exactly `1`; unset or any other value changes
nothing) the command line writes open-time marker lines to standard error for
the open-time harness. This is a release-binary feature, not test support, and
it is documented here only. Each marker is one ASCII line, written and flushed
at once, whether or not standard error is a terminal:

```text
kuru-open-marker v1 <event> <monotonic_ns>
```

`<event>` is `open-start` (before the client's first attach attempt),
`waiting-ownership` (once, and only when the open waited for another copy of
Kuru) or `ready` (where memory becomes ready, including an attach to a running
owner; never written when the open fails). `<monotonic_ns>` is an unsigned
decimal count of nanoseconds from one monotonic clock anchor per process, taken
as the open begins, so `open-start` is near zero and the three events share one
clock. Markers never reach standard output. When the sentence is on the same
terminal, its line is erased before a marker is written and drawn again below
it, so a marker never shares a row with a sentence. A marker that cannot be
written is dropped without affecting the open or the sentences. The first
standard-error line of any kind is `open-start` with markers on, and the
opening sentence without them. Each command's owner retires as soon as it is
unused, so a measurement that repeats commands must await the previous owner's
exit (as `await_owner_exit` does in the command-line tests); otherwise the next
open waits for it and shows the waiting sentence.

With `KURU_OPEN_TIMELINE=1` (exactly `1`; unset or any other value changes
nothing) the project memory service owner records where its own open spends
its time. Like the markers, this is a release-binary feature documented here
only. The owner reads the variable once at its start. On Unix it inherits it
from the command that started it; on Windows the starter forwards it in the
owner's explicit environment only when its own value is exactly `1`. The
engine supervisor, whose environment is cleared, never sees it. The owner
stamps named events as nanosecond offsets from one monotonic anchor, in a
bounded log sealed when its endpoint is published, and writes the log once,
after its close has released the owner lock, to
`memory/services/<hash>/open-timeline-<service-generation>.json` in the data
directory. The write is owner-private, create-only and deliberately not durable
(no sync); a failed write never fails the open, serve or close, and a gated
close returns after that one small write. The file (`format`
`kuru.open-timeline`, `format_version` 1) holds `kuru_version`,
`service_generation`, one wall-clock `anchor_unix_ns`, `events` as
`{event, ns}` pairs, `counts.usage_rows` (the first usage-ledger scan's row
count, or `null`), `dropped` and `late`; never a path, scope, SQL, identity,
credential or content. The events, in canonical order, are `owner-main`,
`owner-lock`, `startup-lock`, `extract-start`, `extract-end`,
`cache-verify-start`, `cache-verify-end`, `create-start`,
`supervisor-spawned`, `channel-accepted` (Windows only), `supervisor-ready`,
`probe-verified`, `template-copied`, `cold-created`, `activated`,
`main-pool`, `version-read`, `migrate-start`, `migrate-end`,
`validate-active`, `candidate-recovery`, `usage-pool`, `usage-bound`,
`usage-scan-1`, `usage-upgrade`, `usage-validate`, `usage-scan-2`,
`usage-record`, `store-ready`, `listener-bound` and `endpoint-published`. An
existing-project open on Unix records all of them except extraction, creation,
activation, migration and `channel-accepted`. Creation, extraction and upgrade
opens add those, and the per-engine-start events (`supervisor-spawned`,
`channel-accepted`, `supervisor-ready`, `probe-verified`) repeat once per
start. `usage-bound` follows the read of the usage head's validation record
(see
[the usage ledger validation record](memory.md#usage-ledger-validation-record)),
and `usage-scan-1` then closes the owned-row walk, which a recorded reopen
skips: its `counts.usage_rows` is 0. Each gated owner run leaves one file of a
few kilobytes. Kuru never reads, lists or removes
these files, so delete them by hand, or measure in a scratch data directory.

A gated owner started with a starter token (every owner Kuru starts) also
streams each stamp as it is taken to `memory/services/<hash>/open-stream-<tag>`,
where the tag is the keyed digest of that token that also names its open
activity record. The file is owner-private and create-only. Each stamp adds
one unsynced line of at most 62 bytes, `<event> <offset-ns> <unix-ns>`, and
nothing else. The owner removes its stream while it holds owner authority:
right after publishing its endpoint and before serving, or when its open
fails, never in close. That is one checked removal, which on Unix syncs the
stream's directory once, as endpoint publication does; it never waits or
retries. It never appends to or removes a stream it did not
create, so a stream left by a killed owner stays until removed by hand.

A starter whose own environment holds the variable exactly `1` reads its own
owner's stream once when its readiness deadline passes and adds an owner
timeline clause before the client phase split:

```text
memory service readiness deadline exceeded; owner timeline: owner-exec=812ms; owner-main=0 owner-lock=3 startup-lock=4 create-start=9930 (ms); last=create-start +9930ms; since-last=20070ms; client phases: ...
```

`owner-exec` is the time from just before the spawn to the owner's first
stamp. The events are offsets in milliseconds from that stamp, `last` is the
owner's last event and `since-last` the time from it to the deadline. A
trailing `skipped=<n>` counts malformed lines. `owner-exec` and `since-last` are
wall-clock differences between two processes, so a clock step skews them.
Instead of events the clause can be exactly `owner timeline: absent` (no
stream: the owner never reached its owner lock, could not create the file, or
published and removed it after the starter's last poll), `empty` (created, no
complete line yet), `stale` (the file's first stamp precedes the spawn: a
predecessor with the same token, possible only when a test reuses a token) or
`unreadable`. An ungated starter reads nothing and its error is unchanged.
Read the phase from `last`: `owner-lock` points at the project startup lock
wait, `extract-start` at extraction, `cache-verify-start` at runtime
verification, `create-start` at preparation before the first engine start,
`supervisor-spawned` without `channel-accepted` at supervisor start-up on
Windows, `channel-accepted` at the engine start inside the supervisor,
`migrate-start` at the migration, and `template-copied` or `cold-created` at
the reap before activation. Many `supervisor-spawned` entries with ordinary
gaps point at repeated engine starts.

The engine supervisor's readiness deadline names the step that had not
completed, under its unchanged outer cause, for example `memory supervisor
readiness deadline exceeded: the supervisor's Ready frame had not completed
32004 ms after the supervisor was spawned: deadline has elapsed`. The step is
`the supervisor's private channel accept` (Windows), `the startup request
write` or `the supervisor's Ready frame`. On Windows a channel accept that
times out on its own timer at the same deadline is reported as the accept
step.

Every coverage partition runs gated: the `coverage:shard` task sets
`KURU_OPEN_TIMELINE=1` in its own environment, and the env-clearing
command-line fixtures forward it beside `LLVM_PROFILE_FILE`. A test that needs
an ungated owner or starter must pin it: `KURU_OPEN_TIMELINE=0` in the owner
environment (`activity::with_owner_environment`) and, for an in-process
starter, `open_timeline::with_gate` or `with_gate_sync`, never the runner's
own environment.

A lost-reply test that pauses a request with the fixture reply pause
(`ReplyPause`, or `test_support::ReplyBarrier` outside kuru-memory), cancels it
and then expects one `reconcile` or recovery call to return a definite answer
cancels once another attachment can read the committed effect. It needs no
reply signal. The owner reports a visible main-view unit or usage ledger
receipt as committed even while the original handler is still registered.
For every other outcome it waits for that handler to settle, without holding
the write guard, and answers from what it reads afterwards. A test that
cancels at `sent` is not covered, because a request the owner never registered
still answers uncertain. Tests of the in-flight answer set the owner's
settlement wait to zero (`set_settlement_wait`), so they observe `InFlight`
without a sleep. Tests that poll `reconcile` until the outcome is definite
also still work.

A record is keyed by the store directory's native identity and birth time, so
it follows a rename, and releasing a root forgets the records beneath it, so a
directory that recycles a removed store's Linux inode does not inherit its
record. A record snapshots the engine-written `server.log` and `endpoint.json`; an engine that starts later in
any process changes them and makes the record stale. The ledger and the
teardown scan run in one critical section, so a teardown's verdict and its
forgetting of the records beneath its root see one consistent ledger. A store that is unreaped, unrecorded or stale keeps the whole root and,
outside an existing failure, fails the test, naming the root, the test and
each store. A test that is already panicking only keeps the root.

Ledger and guard code must finish before its test function returns. Coverage
writes each process's profile at exit, and a thread still inside an
instrumented function then can have its entry counter written without a later
one, leaving a counter expression negative, which the coverage line export
refuses. So a fixture never drops a guarded root on a detached thread: one
that must first wait for a creator process uses
`test_support::release_after_creator_exit`, which waits on the calling thread
and releases or keeps the root before it returns. A fixture that holds
something other than a guarded root until a child exits, such as the Windows
engine fixture's directory handle, waits with `await_creator_exit` on the
dropping thread and keeps both the child and the handle when the wait ends
without an exit. A dropped store owner's reaper thread, which outlives its
owner by design, only sends a report of the reap built from standard-library
calls, and the ledger records it on the next thread that reads the ledger.

A Dolt branch rename or delete must follow server-observed end of every
session on that branch, not only Kuru's pool close; otherwise Dolt refuses the
checked procedure as in use (error 1105). The caller first fences the
branch's pool admission with `Server::fence_pool`; `Server::retire_branch_sessions`
takes that admission, retires the branch's pool, awaits that session end and
returns a `SessionsEnded` proof. The rename, exclusion-probe and delete
procedures are built only from that proof, and each borrows the admission
fence until it has run. The compiler-checked proof is the enforcement: it
cannot be forged outside `server.rs` or reused. Keeping the raw pool close
private is not: `close_pool_without_session_end` is visible to the crate and
closes the pool the same way; it only cannot produce a proof.
`server::branch_procedure_tests` is a textual backstop for raw SQL. It rejects
a quoted rename, delete or force flag (in either quote style, inline, bound,
assigned or on another line) in any non-test source outside
`impl SessionsEnded`, and requires `server.rs` to construct the proof exactly
once, where the session wait returned, but it cannot see a flag or procedure
name assembled at run time.

The lifecycle-ordering measurements are ignored tests, and their Dolt trace is
inert unless `KURU_TEST_DOLT_LOG_DIR` names a directory, so neither runs in
`test`, coverage or CI partitions. Run them on demand with
`mise run //packages/kuru-memory:measure:lifecycle`. Optional
`KURU_TEST_LIFECYCLE_MEASURE_DIR` (default:
`kuru-lifecycle-measurements/m1` under the system temporary directory)
receives their CSV rows, and
`KURU_TEST_LIFECYCLE_MEASURE_ITERATIONS` (default 300) sets the loop count.

To measure how an open changes as a store ages, `mise run
//packages/kuru-memory:measure:age-store --data-dir <dir> --conversations <n>
[--turns <n>] [--seed <n>] [--profile <profile>]` (defaults: one turn, seed 1,
the `dev` profile) grows the one
existing project store under an absolute data directory. Create that store
first with one ordinary `kuru` run against the same data directory; the task
refuses a directory with no store or with several. It waits for any previous
owner to exit, then holds the project's owner lock for the whole run, so no
Kuru command can start an owner on the store meanwhile, and opens the store
directly and offline with the engine already extracted under the data
directory. Each conversation follows a runtime turn's write order through the
memory facade: a new session and its usage marker, then per turn the public
admission, the dispatch journal, the actor's input, one usage invocation
(admit, a terminal observation, settle), the actor's output and the public
settlement. That is `2 + 8 × turns` writes and `1 + 3 × turns` usage-ledger
rows per conversation. The same seed, size and turn count give the same
logical content (identifiers, transcripts, journals and usage numbers); Dolt
commit hashes differ, because commits carry timestamps. Progress goes to
standard error every 500 conversations, and one `kuru.aged-store` JSON line
with the counts and `elapsed_ms` goes to standard output. It is a
measurement aid built only with the package's test support, never part of
`test` or coverage. CI runs it only to build the usage-scan fixture below,
with `--profile release` so one release build serves the whole check.

### Usage scan scaling check

The required CI job `usage-scan-scaling` ("Usage scan scaling (ubuntu-latest,
calibrated bounds)" in `ci.yml`, listed in `ci-gate`) fails when the
usage-ledger startup scan grows faster than linearly with ledger size, or when
a reopen of an already validated ledger decodes any usage row. It runs on
Ubuntu only, because the check is compiled only on Unix.

- **Fixture.** Two aged stores at 1,000 and 5,000 conversations (seed 1, one
  turn). Each holds exactly 4,000 and 20,000 owned usage rows: one session
  marker and three rows per invocation. The binary that ages them writes the
  validation record with every write, so an unforced open of a fixture is
  already a recorded reopen.
- **Forcing.** The measuring driver adds one empty commit without a record to
  the usage branch before each full open, through a direct open of the store
  under the owner lock. That is the head a release that does not write records
  leaves (see
  [the validation record](memory.md#usage-ledger-validation-record)): the next
  open decodes every owned row and records, and the open after it is recorded.
- **Measurement.** The job builds the release kuru-memory test-support
  tooling. For each size it runs one warm-up cycle and then five measured
  cycles. A cycle forces, then starts a cold owner process with
  `KURU_OPEN_TIMELINE=1` for the full open and another for the recorded
  reopen (the warm-up cycle stops after its full open). Each opens the store
  writable, publishes its endpoint, retires and writes its timeline.
- **T(N).** The first usage scan of the full opens, from `usage-pool` to
  `usage-scan-1`. That interval also holds the scan's flat working-set and
  schema checks. The row count is the timeline's `counts.usage_rows`. The
  recorded reopens' own `usage-pool` to `usage-scan-1` interval is printed and
  not bounded: no Ubuntu measurement of it exists yet, and the 0-row check is
  the deterministic guarantee.
- **Checks.** It fails if any of these fails:
  - **rows:** every full open, warm-up included, decoded exactly its size's
    rows (a failure means the forcing step did not leave the head unrecorded);
  - **ratio:** `T(20000) / max(T(4000), 100 ms)` is at most K = 6. Linear
    growth predicts at most 5 and quadratic growth about 17 (25 with no flat
    part). The 100 ms floor keeps a fast small size from turning noise into a
    ratio failure;
  - **ceiling:** `T(20000)` is at most 1 s;
  - **bound-rows:** every recorded reopen decoded 0 usage rows (a failure means
    the record did not bind the reopen).

**Bounds and their derivation.** The numbers below are measured scan-1 medians
of the first four Ubuntu runs of this job, whose bounds were provisional and
whose every open scanned (after the primary-key paging of the scans), as
`T(4000) / T(20000)` in ms: 45.8 / 169.9, 64.3 / 235.8, 51.9 / 177.3 and
56.7 / 212.4. The rest is arithmetic on them.

- **Linear rate.** Two points per run give 7.8, 10.7, 7.8 and 9.7 µs per row
  over a flat part of 14.8, 21.4, 20.6 and 17.8 ms. The worst run is
  10.7 µs per row over 21.4 ms; its prediction at 20,000 rows is
  21.4 + 20,000 × 0.0107 = 236 ms, which is the worst measured T(20000).
- **K = 6.** The unfloored ratio of the four runs is 3.7, 3.7, 3.4 and 3.7
  (at most 3.75), below the ideal 5 because of the flat part. K = 6 is 1.6
  times the worst 3.75. A quadratic scan scales the variable part by 25, so
  the worst run predicts 21.4 + 25 × (64.3 − 21.4) = 1,094 ms, a ratio of 17,
  nearly three times K.
- **Floor.** Every Ubuntu T(4000) is below the 100 ms floor, so the ratio check
  is in practice `T(20000) <= 6 × 100 ms = 600 ms`, 2.5 times the worst 236 ms.
  It is the binding check there.
- **Ceiling = 1 s.** 4.2 times the worst 236 ms, below the quadratic
  prediction of 1,094 ms and 30 times under the 30 s open deadline. It binds
  only when T(4000) exceeds about 167 ms (1,000 / 6), on a runner about 2.6
  times slower than the slowest measured (64.3 ms).
- **To recalibrate**, rerun the job on Ubuntu, take the worst of the printed
  `T(4000) / T(20000)` pairs, repeat the arithmetic and change `CALIBRATED` and
  `DERIVATION` in `test_support/usage_scan.rs` together: the report prints both.
  An engine upgrade can change a table's content hash, which reads as a missing
  record and costs one full open; the forcing step makes the check independent
  of it.

**Output.** Every run prints the per-size scan medians and samples of the full
opens and of the recorded reopens, the second scan, the usage block, the whole
open, the ratio, the bounds and their derivation, and appends them to the job
summary. A failed assertion also writes every
timeline to standard error. The records (`records.jsonl`, `verdict.json`),
timelines, owner logs and ageing logs are uploaded as
`ci-usage-scan-attempt-<n>` on any outcome.

**The fixture is aged in-job on every run and never cached.**
- **Rule.** The fixture may occupy the repository's shared Actions cache
  only if, after `CALL DOLT_GC()` on each store, both stores together stay
  under about 1 GB on disk.
- **Measured (2026-10-01).** On macOS arm64, freshly aged stores measured
  with `du -sk data-<n>/memory`:

  | Stores | Before GC | After `DOLT_GC()` | After `DOLT_GC('--full')` |
  | --- | --- | --- | --- |
  | 1k | 259,908 KiB | 222,420 KiB | 209,212 KiB |
  | 5k | 1,450,004 KiB | 1,425,852 KiB | 1,366,200 KiB |
  | Total | 1.63 GiB | 1.57 GiB (1.69 GB) | 1.50 GiB |

  Almost all of it is live history in `noms/oldgen`. The engine already runs
  with automatic GC, so a manual GC reclaims only 2 to 4%. Before GC, the
  ubuntu-latest stores are larger still: 335 to 340 MB at 1k and 2.1 GB at
  5k. The total is over the budget, so the job keeps no cache entry for the
  fixture.
- **Stored size.** The one entry `main` saved before this decision held the
  pre-GC Ubuntu fixture in 495,129,806 bytes, compressed. At the time, the
  repository's caches held 63.79 GB in 490 entries; before that save,
  61.84 GB in 475. The decision applies the rule to on-disk size, not to
  that compressed size.
- **Cost.** Every run creates and ages both stores, one after the other.
  `main`'s first run took 13 min 40 s for the whole job. Of that, 171 s went
  to the release build and 7 min 41 s to ageing (5k: 380,384 ms). That stays
  under the 14 to 17 minutes of the slowest native partition.
- **Seal.** `fixture.json` still records a key over the compiled constants
  that decide what the stores measure. These are the fixture format, the
  seed, turns and sizes, the aged-store report format, `CURRENT_VERSION` and
  `USAGE_CURRENT_VERSION`, the bundled Dolt version, digests and target, and
  the root's canonical path. A measurement therefore refuses a reused local
  root sealed by another build or moved elsewhere. The job pins the root at
  `$RUNNER_TEMP/kuru-usage-scan`, because an owner binds a store's scope to
  its project's canonical path.

**Decision rule.** The job prints each size's ageing time. If cold 5k ageing
on Ubuntu exceeds 20 minutes, the fixture switches to a bulk-seeded ledger
written with the ledger's own encoders in batched commits.

**Running it locally** on Unix, with an absolute scratch root `<root>`:

```sh
mise run //packages/kuru-memory:measure:usage-scan:fixture -- create --root <root> --conversations 1000
mise run //packages/kuru-memory:measure:usage-scan:fixture -- create --root <root> --conversations 5000
mise run //packages/kuru-memory:measure:age-store -- --profile release --data-dir <root>/data-1000 --conversations 1000 > <root>/age-1000.json
mise run //packages/kuru-memory:measure:age-store -- --profile release --data-dir <root>/data-5000 --conversations 5000 > <root>/age-5000.json
mise run //packages/kuru-memory:measure:usage-scan:fixture -- seal --root <root>
mise run //packages/kuru-memory:measure:usage-scan -- --root <root> --evidence <dir> --samples 5 --assert
```

`create` refuses to run with `KURU_OPEN_TIMELINE` set, and `seal` refuses a
store that holds a timeline. `measure:usage-scan` sets the variable itself.

Every measured open leaves its timeline file in the store's services
directory: as the timeline section above notes, nothing in Kuru removes them.
The driver reads each open's file by its service generation, so files left by
earlier runs never mix into a measurement. Still, measure a scratch root that
is never sealed or cached again, or delete the files by hand.

`store::engine_contract_tests` pins the Dolt behaviours that creating stores
from a pre-migrated template relies on: root creation and bootstrap on a
copied data directory, identity commits on two refs, main-pool reads of
branch state and history, commit hash shape and the bytes a stopped data
tree holds. They run in the ordinary `test` task. Their cross-OS case runs
only when `KURU_ENGINE_CONTRACT_CROSS_OS_CAPTURE` names a capture directory
(`capture.json` beside a `data/` tree) produced on another operating system;
otherwise it prints and asserts a not-run result. Setting
`KURU_ENGINE_CONTRACT_REQUIRE_CROSS_OS=1` turns that not-run result into a
failure, for a job that consumes such a capture.

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

## Lint configuration and Windows-target lint

The root `clippy.toml` is the workspace's only Clippy configuration. Clippy
1.98.1 searches `CLIPPY_CONF_DIR`, else the package directory, and walks up to
the first directory holding `clippy.toml` or `.clippy.toml`
([Clippy 1.98.1 configuration](https://github.com/rust-lang/rust/blob/1.98.1/src/tools/clippy/book/src/configuration.md)).
A package-level file would therefore replace the root file and drop its bans.

The file bans methods through
[`disallowed-methods`](https://rust-lang.github.io/rust-clippy/rust-1.98.0/index.html#disallowed_methods).
Every entry carries a `reason` that names the safe alternative. The first bans
cover thread-scoped and ad hoc global tracing subscriber installation:

- `tracing::subscriber::set_default` and `with_default`;
- `tracing::dispatcher::set_default` and `with_default`;
- `tracing_subscriber::util::SubscriberInitExt::set_default`;
- the global installers `set_global_default`, `SubscriberInitExt::try_init`
  and `SubscriberInitExt::init`.

tracing-core caches each callsite's interest for the whole process. A
subscriber set on one thread can miss a callsite that another thread, holding
no subscriber, registered first; PR #126's first capture failed that way.
Observe events through one process-wide recorder installed once, as
`kuru-memory`'s `RetainedStageRecorder` does. The application's one global
subscriber is installed at startup by `apps/kuru-tui` `diagnostics::install`.

`disallowed_methods` is a single lint in Clippy's `style` group. An allowance
of it, of `clippy::style`, of `clippy::all` or of `warnings` switches off every
ban at once. Allow a reviewed call site only on its own statement, with
`expect`, which fails once the call is gone, and a reason:

```rust
#[expect(clippy::disallowed_methods, reason = "why this site is safe")]
tracing::subscriber::with_default(subscriber, || { /* ... */ });
```

The repository check in `lint:tooling` rejects:

- a `clippy.toml` or `.clippy.toml` under `apps/` or `packages/`;
- an inner `allow` or `expect` of those lints anywhere, and one on a `mod`,
  `fn`, `impl` or `trait` item;
- an outer `allow` of those lints at any scope, and an `expect` without
  `reason = "..."`;
- a `[lints]` or `[workspace.lints]` table that allows or expects them;
- `CLIPPY_CONF_DIR`, `--cap-lints`, or a command-line `-A` of those lints in
  mise, Cargo or workflow configuration: the root `mise.toml` and
  `.cargo/config{,.toml}`, each package's `mise.toml`, any
  `.cargo/config{,.toml}` under `apps/` or `packages/`, and the workflows.
  Command words split at `=` as well, so `RUSTFLAGS=-Aclippy::style` inside
  a command is found;
- a root entry without a reason, and a missing required ban.

It reads text, so an allowance produced by a macro, or lint flags from outside
the repository, stay outside it.

Rust compiles `cfg(windows)` items only for a Windows target, so host `lint`
never sees them. Every package with such code owns a `lint:windows` task. It
runs Clippy for `x86_64-pc-windows-msvc` with warnings as errors, and the root
`lint:windows` aggregates those tasks. Install the target first with
`mise run setup`, or with
`rustup target add x86_64-pc-windows-msvc --toolchain 1.98.1`.

Off Windows, the tasks point `CC_x86_64_pc_windows_msvc` and
`AR_x86_64_pc_windows_msvc` at stand-ins in `packages/kuru-delivery/support`.
The stand-ins write empty objects, so C build scripts such as `aws-lc-sys`,
`ring` and `libsqlite3-sys` finish. Clippy never links, so no object is
consumed. The stand-in compiles no C, and every compile and probe succeeds; it
answers preprocessor probes with `clang`, so a build script that decides by
probing may set different cfgs than real MSVC would. Only Rust diagnostics are
checked; the native Windows jobs remain the build proof. On a Windows host the
tasks use the native MSVC toolchain.

`kuru-memory` and its dependents need the verified Windows engine archive,
which their tasks prepare first. CI's `Lint (x86_64-pc-windows-msvc)` job runs
on Ubuntu after `bundle-inputs`. It imports the run's verified archives with
`--archive --offline`, and `ci-gate` requires it. The job checks Rust
diagnostics only. The native-tests Windows jobs prove that the code compiles,
links and behaves on Windows. hk does not run `lint:windows` before a push;
run it locally when changing Windows code.

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
activated shells or mise shims. It needs mise 2026.9.13 or later, the root
`min_version` hard floor (2026.9.18, which CI runs, is recommended). mbx stores compiled outputs in one content-addressed cache and
restores matching compilations into each checkout's own `target/`, so a new
worktree mostly restores its dependencies instead of recompiling them while
concurrent worktrees keep separate Cargo locks. Calling rustup's `cargo`
directly bypasses the wrapper; use `mise exec -- cargo` from editors and agents.

The root `min_version` is a table because the two floors answer different
questions. The hard floor stays at 2026.9.13: a hard floor at the latest
release would make a developer machine on the previous Homebrew stable refuse
every mise task, including the git hooks. The soft floor tracks the latest
release (currently 2026.9.18, the CI pin), so an older local mise gets a
recommendation to upgrade rather than a refusal.
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

`KURU_MBX=0` does not remove mr-boxington from the configured tool set, so CI
also keeps it from being downloaded. Every workflow that installs tools sets
`MISE_EXEC_AUTO_INSTALL=false` at workflow level. On Windows, mise's executable
shims run `mise x`, which would otherwise install every missing configured tool,
mr-boxington included, on the first shim call such as `rustup` or `cargo`.
`MISE_TASK_RUN_AUTO_INSTALL=false` stops `mise run` doing the same before a
task. Every workflow, the release workflow included, sets both at workflow
level, so every job installs only its `mise-action` `install_args` and the
tools its named install tasks select. A job that needs another tool must name
it in `install_args`; the source installers already install only `rust` and
pass it explicitly to `mise exec`.

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

On Windows on Arm, import the source-built archive (a `Bundle build` artifact or
a linux-x64 build; see [source-built engine inputs](#source-built-engine-inputs))
the same way and build for the Arm target:

```powershell
$env:KURU_DOLT_BUNDLE_DIR = 'C:\BuildInputs\kuru'
mise run //packages/kuru-memory:bundle:prepare -- --target aarch64-pc-windows-msvc --archive C:\Downloads\dolt-windows-arm64.zip --offline
$env:KURU_DOLT_BUNDLE_OFFLINE = 'true'
$env:CARGO_NET_OFFLINE = 'true'
mise run //apps/kuru-tui:build:release -- --target aarch64-pc-windows-msvc
```

`verify:windows-imports` finds `dumpbin.exe` through `vswhere` under either the
x64 or the ARM64 MSVC host tools.

The shipping binary is under `target/<target>/release/kuru.exe`
unless `CARGO_TARGET_DIR` selects another target directory. For its native import
check, set `KURU_EMBEDDED_TEST_BINARY` to that absolute path and run
`mise run //apps/kuru-tui:verify:windows-imports`. This maintainer check uses MSVC
tools; the installed application does not.

CI's source-install smoke disables both Cargo network access and missing-bundle
downloads after preparing the dependencies. On Windows, the memory-owned
`mise run //packages/kuru-memory:bundle:verify-native-build` task runs the
shipping `build:release` command with `--offline` three times. First, with the
caller's own mirror selection, it requires the installed copy to be the fresh
output of that identical command: Cargo must compile nothing, and its
`release/kuru.exe`, which the check only hashes, must match the installed
SHA-256. It then invokes the actual Cargo build with isolated missing and
same-size corrupt mirrors and requires the specific build-script rejection. Run
it after source installation, in the same environment, with
`KURU_EMBEDDED_TEST_BINARY` pointing to the installed copy outside Cargo's output
directory. It verifies that the installed executable and original prepared
archive retain their hashes. The rejected builds leave the memory build-script
fingerprint dirty, so the next build of that target directory reruns the build
script and recompiles kuru-memory and its dependents.

### Source-built engine inputs

Upstream Dolt publishes no archive for some targets, currently
`aarch64-pc-windows-msvc`, and cannot build them without cgo. A `built` manifest
asset therefore pins everything its archive is made from: the Dolt Go module
version and its `go.sum` `h1:` hash, the ICU source tarball's size and SHA-256,
the Go and llvm-mingw toolchain versions, the recipe name and the single build
host. Preparing, compiling or provisioning any other target never fetches,
builds or reads a built asset's inputs. Runtime provisioning never builds or
downloads an engine.

`//packages/kuru-memory:bundle:test-fixtures`, a dependency of the memory
tests, always prepares the upstream `x86_64-pc-windows-msvc` ZIP, on every host
including Windows on Arm: it is a host-independent fixture for the archive
decoder, not an engine for the host.

The build host is **linux-x64 only**, including for the Windows on Arm engine
that CI and release jobs import; no Windows or Arm runner builds it. llvm-mingw's target runtimes embed paths
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

Workflows that need a built archive call the same workflow through
`workflow_call`, passing the commit as `ref`. That path does not repeat the
two-build proof: it restores an archive cached under the pinned digest or builds
once, fails unless the bytes match the committed pin, and uploads the
`bundle-input-<target>` artifact. Consuming jobs import it with
`bundle:prepare -- --target <target> --archive <file> --offline` into their own
private `KURU_DOLT_BUNDLE_DIR`, which checks the pin again.

The cache key is the target, the pinned digest and a hash of the recipe inputs
that decide the archive bytes. A recipe change therefore rebuilds once and
proves the recipe still reproduces the pin. The key hashes:

- the asset manifest `packages/kuru-memory/support/dolt-assets.json`, which pins
  the sources and the Go and llvm-mingw toolchains;
- the helper's `main.rs`, `bundle.rs` and `bundle/build.rs`. The helper's test
  modules never reach the binary;
- the ZIP writer, `packages/kuru-archive/src/zip.rs`;
- the `Cargo.lock` records of kuru-archive's locked dependency closure, which
  holds the ZIP and deflate crates. The job's `ENGINE_ARCHIVE_CRATES` names them.
  Other `Cargo.lock` changes do not rotate the key.

`packages/kuru-memory/mise.toml` and `mise.lock` are not hashed. Instead,
`cargo test -p kuru-delivery --features tooling --test bundle_build` requires
the build configuration to equal what was reviewed, so a change fails the test
rather than reusing a stale archive. It checks:

- the key: exactly the target, the pin, one `hashFiles` list and the crate
  digest. The list must equal the inputs derived from the helper's CLI default
  manifest, its module declarations and the modules its keyed files use, and
  `ENGINE_ARCHIVE_CRATES` must equal the closure recomputed from `Cargo.lock`;
- the `bundle:build` task: its exact command and directory, no keys besides
  `description`, `dir`, `env` (only unset entries), `tools` and `run`, and each
  tool locked with the manifest's URL and digest. `setup:build-tools` must
  install exactly those tool assets;
- the mise configuration the task loads. The memory package's file may hold
  only `[vars]`, `[tasks]` and a reviewed `[env]`. The root file's `[env]` and
  `[tools]` names, settings, tool aliases, monorepo roots and hooks are fixed.
  No other tracked mise config file (`mise.local.toml`, `mise.<env>.toml`,
  `.mise.toml`, `mise/`, `.mise/`, `.config/mise*`, `.tool-versions`) may exist
  in the root, `packages/` or the memory package;
- the Cargo configuration the helper's build loads. No tracked
  `.cargo/config` or `.cargo/config.toml` may exist in the root, where the task
  runs Cargo, nor in `packages/`, the helper's package or the memory package.
  Its `[env]` (including a forced `PATH`, which reaches the ICU build),
  `build.rustflags` or a target `runner` would change the build without
  changing the key;
- the input job: the workflow environment, its keys and `ubuntu-latest` runner,
  its actions, step environments, no `GITHUB_ENV` or `GITHUB_PATH` writes, and
  the exact toolchain and build steps;
- the keyed files: no `include_str!`, `include_bytes!`, `include!` or
  `super::super`, and no crate or module outside the key except the listed
  exemptions below;
- the compression crates' Cargo features: the workspace specifications of
  `flate2`, `zip` and `crc32fast`, plain `workspace = true` use in every member
  with no member feature enabling theirs, no `[patch]` or `[replace]`, resolver
  3, and the registry packages in `Cargo.lock` that depend on them. A feature
  can switch the deflate backend without changing `Cargo.lock`. The check is
  static, so it needs no Cargo child process and gives the same answer on every
  host.

Some inputs are outside both the key and the test:

- the helper's `crate::command`, `crate::archive`, `crate::lease` and
  `crate::staging` modules, and `kuru-platform`. `command` decides a child's
  environment; the recipe clears it to `PATH` and `HOME`;
- the modules `main.rs` dispatches only for other subcommands;
- the root mise tool versions, including the Rust toolchain that compiles the
  helper, the pinned mise version, and Cargo profiles;
- the runner image's own `sh`, `make`, `bash` and host C compiler, which build
  ICU. Only the Go and clang version strings are checked.

The only required check behind these is the pin check. The determinism job
(two builds on a pull request that touches a listed recipe path) also catches a
change, but it is not a required check. The key only decides when to rebuild. A
restored or rebuilt archive is used only after it matches the pinned size and
SHA-256. A restored mismatch is discarded and rebuilt, and a rebuilt mismatch
fails the job. Only `main` saves the cache.

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
mise run format:code ::: lint:rust ::: lint:windows ::: typecheck ::: coverage ::: lint:tooling ::: cospec:validate ::: cospec:managed:check ::: docs:check
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

## Memory pool acquire timeouts

Every store pool is a `MemoryPool` (`packages/kuru-memory/src/pool.rs`). Each
statement returns its connection to the pool before its future completes, so
sequential work reuses one authenticated session. A receipt-bearing write
returns its session only after a receipted success; any other outcome ends the
session before the uncertain-write fence reconciles it. A new connection runs
TCP, MySQL authentication and the identity callback inside the acquisition
that opens it.

### Bounds

Once memory is open, an acquisition is bounded by the remaining budget of the
statement or operation it serves, never by a shorter window of its own:

- **Statement budget.** Memory statement and operation budgets run in a
  budget scope, `pool::within(budget, work)` or
  `pool::within_until(deadline, work)`, which behave like
  `tokio::time::timeout` and `timeout_at`. A nested scope ends at the
  earliest enclosing deadline. The pool funnel registers its pending
  acquisition with the enclosing scope, so a budget that runs out while the
  acquisition waits is reported as that acquisition's timeout.
- **Pool ceiling.** Every store pool's lifetime SQLx `acquire_timeout` is
  `QUERY_TIMEOUT` (30 s), the memory statement budget. It decides only an
  acquisition outside any scope, or one whose scope deadline falls on the same
  timer tick; nothing is set above the budget it serves.
- **Write budget.** A receipt-bearing write takes one deadline,
  `QUERY_TIMEOUT`, before its pool acquisition and spends it on the
  acquisition, the `CONNECTION_ID()` identity statement, any validation
  before its pending record (a session fork's source traversal), the write
  and the session's return, so these end within one `QUERY_TIMEOUT` from
  before the acquisition. A store mutation, a session catalog write, a
  candidate creation and a usage ledger change take that deadline as soon as
  they hold the store's write lock, so it also covers every read before their
  pending record (reconciling an earlier uncertain write, the schema check, a
  logical receipt's match and its recorded outcome, a candidate creation's
  ref and base reads) and, for a candidate creation, the new branch's pool
  creation. A candidate promotion merge, status transition, deletion and
  session exclusion, and the usage ledger's validation record, take their
  deadline at their acquisition; their earlier reads (promotion's
  reconciliation, ref, revision and working-set reads, session retirement,
  the validation record's bound check) keep their own statement budgets. A
  contended acquisition therefore uses write time: a write that then runs out
  after its pending record is an uncertain write, reconciled by the existing
  fence. An acquisition that fails returns before any statement and before
  the pending record, so it never makes a write uncertain.

  The client waits `OPERATION_TIMEOUT` (35 s, `QUERY_TIMEOUT` plus
  `REPLY_MARGIN` in `service/rpc.rs`) for a reply. A write's own budget fits
  that wait, but a service write as a whole is not bounded by it: the wait
  for the write lock (the previous write's end), the earlier reads of the
  writers that take their deadline at the acquisition, reconciliation after
  a write that ends without its receipt (the fence's own path), and candidate
  promotion, abandonment and cleanup, which run session retirement, pool
  retirement and several writes, each under its own write budget, all add to
  it. Past the client's wait, the outcome query (see
  [memory service protocol](#memory-service-protocol)) and the uncertain-write
  fence recover the write's outcome.
- **Creation budget.** A pool created after memory is open (a candidate's or
  other branch's pool) runs its first acquisition, its first connection's
  return and identity verification under one `QUERY_TIMEOUT` budget, nested
  in any enclosing scope. If that budget runs out, creation fails with
  `memory branch pool creation budget elapsed` and no pool is retained;
  nothing is retried. `Server::pool` holds the server's pool map while it
  creates a pool, so a stalled creation delays other `Server::pool` calls for
  up to that budget.
- **Read transactions.** `MemoryPool::begin` runs its acquisition and `BEGIN`
  under one `QUERY_TIMEOUT` scope; each statement in the transaction keeps its
  own budget.
- **Opening.** While memory is opening, a pool's first acquisition keeps the
  bound derived from the startup budget (`startup_timeout_secs`), never less
  than `OPENING_POOL_FLOOR` (2 s), with its first-connection release cut and
  retry. With the 30 s ceiling, that retry is reachable only when
  `startup_timeout_secs` exceeds 30. The attach probe and its identity check
  keep their 2 s.

Use `pool::within`/`within_until` for memory statement and operation
budgets in `packages/kuru-memory`, not `tokio::time::timeout`, so an expiry
during an acquisition is typed. A tokio timeout still cancels the work at its
deadline, but the funnel cannot see it, so its expiry carries no cause. These
stay tokio timeouts: service IPC frame timeouts, the outcome probe
(`PROBE_BUDGET`, 2 s, positive evidence only) and the four guarded outcome
reads in `service/rpc.rs` (their expiry is answered as still uncertain without
its cause), the `DREAM_LEASE_WAIT` lease waits in `facade.rs`, pool closes
(`CLOSE_GRACE` and the migration pool closes) and process or supervisor
timeouts. A scope is task-local: work moved into `tokio::spawn`, or a stream
polled after its scope ended, falls back to the pool ceiling.

### Diagnostics

An acquisition that reaches its bound fails with a `PoolAcquireTimedOut`
diagnostic instead of SQLx's bare `pool timed out while waiting for an open
connection`. It names the bound that ended the wait, that budget and what was
left of it when the acquisition began, for example:

```text
memory pool acquire on kuru/main timed out after 0.501 s (statement budget 0.500 s, 0.500 s left when the acquire began) waiting for a connection held by Kuru work (every permit held); pool size 4 of 4, 0 idle, 4 checked out; 4 connections authenticated since the pool opened, 0 during this wait
```

With the pool ceiling as the bound the parenthesis reads
`(pool ceiling 30.000 s)`.

The connection counts are of connections that entered Kuru's identity
callback, which runs after TCP and MySQL authentication have finished. A
handshake that never finishes is not counted. The wait class says what to
investigate:

- `every permit held`: Kuru work held every connection. Look for contention
  above the pool's maximum or a holder that did not finish. The checked-out
  count covers sessions Kuru work still holds; a session stops counting once
  its release starts, including a release cancelled mid-flight, which SQLx
  closes itself.
- `a new connection in Kuru's identity callback`: a new connection reached
  the identity callback during the wait. The phase shows how far through the
  callback's queries it got.
- `no new connection reaching Kuru's identity callback`: capacity existed and
  no new connection reached the callback, so the wait was SQLx's ping of an
  idle connection, a release still in flight, or a TCP or MySQL handshake that
  did not finish. Kuru cannot tell these apart, because the callback is the
  first point at which it observes a new connection. No phase is printed,
  because the pool's latest phase would be stale.

The diagnostic contains no SQL text, credentials, endpoints or paths. Where it
appears in an error chain:

- **Statement budget.** A `pool::BudgetElapsed` (`memory statement budget of
  0.500 s elapsed while acquiring a pool session`) whose source is the
  diagnostic. SQLx did not time out, so its `PoolTimedOut` is not in the
  chain.
- **Pool ceiling, through the pool's own `acquire`.** Context over SQLx's
  `PoolTimedOut`.
- **Pool ceiling, through a statement.** A `sqlx::Error::Io` of kind `Other`,
  which reads `error communicating with database: memory pool acquire ...`.
- **Budget elapsed during execution.** A `BudgetElapsed` with no source
  (`memory statement budget of 30.000 s elapsed`). It is not an acquisition
  timeout, and `pool_acquire_timeout` returns `None` for it.

Use `kuru_memory::pool::pool_acquire_timeout` to find the diagnostic in an
error chain. Callers keep their own context strings, such as `memory read
deadline exceeded`.

The pool logs each timeout as a `memory pool acquire timed out` warning with
the same fields. An acquisition still pending after
`SLOW_ACQUIRE_THRESHOLD` (2 s) logs one `memory pool acquire still waiting`
warning with the same fields plus the remaining budget and the number of
pending acquisitions, and one that then succeeds logs `memory pool acquire
completed slowly` with its wait. The threshold only logs; it never ends an
acquisition. A post-open pool creation acquires on the raw SQLx pool before a
`MemoryPool` exists, so it leaves no slow record; it fails with its connection
phase or as its creation budget's `BudgetElapsed`.

### Identity rejection and failed connects

An authored identity rejection (data directory or project/instance mismatch)
ends the acquisition at once with its cause, never as a timeout, in pool
creation and on a retained pool. It is sticky: every later acquisition on that
pool fails with the same cause, including one that would have reused an idle
session, until the store is reopened. The same endpoint cannot answer
differently, and a new pool still authenticates every new connection.

SQLx retries other connection failures inside one acquisition until its
bound, with backoff capped at a fifth of the time left: a refused TCP connect
(for example, an engine that is gone) or a non-identity callback error. With
a 30 s bound, such a terminal fault is reported as the acquisition's timeout
at its bound rather than after 2 s.

## Memory service protocol

Each typed memory service operation (`ServiceCall`, `ViewOperation` and
`LedgerOperation` in `packages/kuru-memory/src/service/rpc.rs`) has one entry in
its exhaustive `contract()` match. The entry states whether a lost reply may
hide a write, which durable receipt proves the outcome, and the reply budget.
An outcome query may wait for the original handler to settle, within
`OPERATION_TIMEOUT` minus a reply margin (`REPLY_MARGIN`), and it ends the wait
as soon as its client disconnects or cancels. The wait relies on the same
assumption as the uncertain-write fence: Dolt removes a session's process-list
entry only after the session's running command returns. The store test
`receipt_is_hidden_before_dolt_commit_and_durable_without_sql_commit` measures
the receipt visibility that the lock-free probe depends on.
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
pinned. Its optional starter token is additive, skipped when absent, and
needs no `PROTOCOL_MINOR` bump (see the starter token below). Shared `kuru-core` types that
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
tool pins with the matching five-platform lock refresh:

```sh
mise lock --platform linux-x64,linux-arm64,macos-arm64,windows-x64,windows-arm64
mise -C apps/kuru-docs lock --platform linux-x64,linux-arm64,macos-arm64,windows-x64,windows-arm64
mise -C packages/kuru-delivery lock --platform linux-x64,linux-arm64,macos-arm64,windows-x64,windows-arm64
```

Root `mise.toml` pins the standalone `github:aligned-team/cospec` tool in
table form with `[tools."github:aligned-team/cospec".platforms.windows-arm64]
asset_pattern = "cospec-*-windows-x64.zip"`: cospec publishes no
`windows-arm64` release asset, so this pins the lock's `windows-arm64` entry to
the existing `windows-x64` asset (`asset_pattern` replaces mise's asset
autodetection for that platform). Every other tool listed here resolves its
own `windows-arm64` entry without a pin. Three of those entries are x64
executables run under emulation. `aqua:cocogitto/cocogitto` and
`aqua:taiki-e/cargo-llvm-cov` fall back to their `windows-x64` assets through
the aqua registry's own `windows_arm_emulation` flag. For cargo-llvm-cov this
holds although upstream also publishes `cargo-llvm-cov-aarch64-pc-windows-msvc`
archives: the aqua backend offers no per-platform asset option (only
`symlink_bins`, `vars` and `prerelease`; see mise's
[aqua backend](https://mise.jdx.dev/dev-tools/backends/aqua.html)), so the root
and delivery locks record `cargo-llvm-cov-x86_64-pc-windows-msvc.tar.gz` for
`windows-arm64`. `aqua:koalaman/shellcheck` publishes a single Windows archive,
so its `windows-arm64` entry is the same x64 `shellcheck-v0.11.0.zip` as its
`windows-x64` entry. The rest resolve native Arm64 Windows assets, each under
its project's own naming (for example `hk-aarch64-pc-windows-msvc.zip`,
`actionlint_1.7.12_windows_arm64.zip`, `taplo-windows-aarch64.zip` and
`node-v26.10.0-win-arm64.zip`).

A pattern that matches no asset does not make `mise lock` fail: it reports the
platform as skipped, exits successfully and writes no `windows-arm64` entry for
cospec, and only a later `MISE_LOCKED=1` installation fails.

Mise records a per-platform option as a second
`[[tools."github:aligned-team/cospec"]]` lock element carrying
`options.asset_pattern`, its own `specifiers` and the `windows-arm64` row (see
mise's [lockfile format](https://mise.jdx.dev/dev-tools/mise-lock.html), where
one version can have several entries distinguished by `options`). Unlocked
installs (including a reinstall of an installed tool) and `mise exec` or
`mise run` auto-installs rewrite each per-project lockfile from the host's
resolved toolset. They drop that element's `specifiers`, add rows for other
host variants (`macos-x64` rows on an Arm64 Mac), and, when the GitHub API refuses the
query, may omit the `windows-arm64` row. Only `mise lock` restores it. This
behavior is the same in mise 2026.9.4, 2026.9.13 and 2026.9.18. Repository
tasks that install tools therefore install with `--locked`: the docs app's
`setup:tools` (`mise install --locked node npm`, which `format:check`,
`docs:*` and the hk hooks reach) and the delivery package's cargo-audit setup.
The hk hooks run without `MISE_LOCKED=1`. Run ad hoc installs or `mise exec`
outside an intentional lock refresh with `MISE_LOCKED=1`, which stops
automatic lockfile updates and fails on a missing entry instead of resolving it
([strict lockfile mode](https://mise.jdx.dev/dev-tools/mise-lock.html)).
The refresh commands above still write with `MISE_LOCKED=1` set. CI
already exports `MISE_LOCKED=1`. The repository check in `lint:tooling`
(`mise run //packages/kuru-delivery:check:repo`) fails unless the lock holds, for
each root tool with a `platforms` table, an element with those options, the
pinned version in `specifiers` and a `checksum` and `url` for that platform.
Run it after refreshing the root lock:

```sh
MISE_LOCKED=1 mise run //packages/kuru-delivery:check:repo
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
MISE_OS=windows MISE_ARCH=aarch64 mise -C packages/kuru-delivery lock github:jdx/communique --platform windows-arm64
```

Review the resulting `provenance_verified` metadata alongside URLs and checksums.
This also keeps CI installation from creating uncommitted verification metadata.
Run these lock refreshes with the CI-pinned mise version (2026.9.18) without
`--upgrade`: per the 2026.9.7 and 2026.9.16 release notes, lockfile revisions 2
and 3 are unreadable by older mise, including the root `min_version` hard floor. mise 2026.9.18 keeps
existing `provenance_verified` lines but did not add one to a cospec entry
from which they were removed, where 2026.9.4 did. CI installs with
`MISE_LOCKED=1` and never rewrites the lock, and `check:repo` reads no
provenance field, so that difference does not fail CI.
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
the embedded engine into the shared test cache, then builds or verifies this
build's store template beside it. Its build dependency prepares
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
