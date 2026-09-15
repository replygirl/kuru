# Development

Install pinned tooling with `mise install`, then run `mise run setup`. Rust 1.98.1
is declared in both mise and rust-toolchain.toml. Cargo.lock pins runtime
transitives. The [dependency audit](dependencies.md) records latest stable
versions and the exact upstream constraints on transitive updates. mise.lock contains platform-specific tool URLs and checksums.
Cospec is a standalone executable with embedded OpenSpec. Its validate/apply
JSON and managed-file checks run without a project OpenSpec dependency. The
pinned 0.7.1 release still needs the compatibility fix for its embedded
OpenSpec 1.11.0 bundle: duplicate entrypoint execution makes the unpatched
instructions command fail to return one JSON document. The cospec mise task scopes a small
[compatibility preload](../packages/kuru-delivery/support/cospec-preload.cjs) to
that exact bundle hash using cospec's own runtime. It preserves command arguments
and the original gate; standalone contract tests cover clear, hard-blocked,
soft-blocked and missing-artifact outcomes. Remove the preload only after an
upstream release fixes vendoring and passes those tests without it. The 0.7.1
archive-gate corrections do not satisfy that removal condition.

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
to install only those two exact package-owned pins. Communiqué 1.3.5 provides Linux x86_64
and arm64, macOS arm64 and Windows x86_64 binaries; it does not publish an Intel macOS binary.
Run the full maintainer gate on one of those supported platforms. Building,
installing and packaging Kuru on Intel macOS uses the Rust tasks and does not
require Communiqué or maintainer setup.

CI runs format, lint, typecheck, repository/workflow tooling, cospec validation,
managed-file checks and documentation as separate Ubuntu jobs. Native coverage
runs as one workspace suite on Linux x86_64 and macOS arm64. Windows x86_64
runs four package shards in parallel, validates their exact source, toolchain,
artifact inventory, Cargo-native runner ledger and raw-profile receipts, and
then enforces one 90% workspace report. Each shard compiles the same full
workspace/all-target/all-feature graph; its task-private runner executes only
the assigned standard test targets while Cargo retains package cwd and runtime
environment. Its source installation and installed offline-runtime checks run beside
the coverage shards after independently preparing their locked inputs. Linux Clippy does not
analyze platform-specific conditional code; the native suites compile and test
those branches. Intel macOS and Linux arm64 additionally build and package the
native executable, exercise real memory and verify the packaged offline runtime.
Windows primitives retain a separate native coverage job for early feedback.
The required `ci-gate` accepts only success from every branch of this graph.

Ubuntu's coverage step disables Rust test-profile debug information so its
instrumented Kuru executable remains a valid input to the same production
release-archive bound exercised by the packaged-runtime fixture. Coverage maps,
the full test graph, and the 90% line threshold remain enabled. Panic text is
retained, but Ubuntu coverage backtraces may omit source file and line details;
use a focused local run or another native job when those details are needed.

CI installs only each job's tools before task activation, disables automatic
installation of unrelated root tools, and uses `MISE_NO_HOOKS=1` because validation jobs do not create Git
commits. Local Git hooks and maintainer setup retain hk. Archive-only Intel macOS
jobs need neither hk nor Communiqué, which have no matching upstream binaries.

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
| `mise run coverage` | Run the behavioral suite under LLVM instrumentation, minimum 90% workspace line coverage |
| `mise run test:install` | Native archive tests and, on macOS/Linux, real Bash bootstrap tests |
| `mise run //packages/kuru-delivery:test` | Delivery contracts, including native PowerShell bootstrap/update fixtures on Windows |
| `mise run //apps/kuru-tui:test:embedded-runtime` | Package, install, update and reopen actual Kuru with cold offline memory |
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
run already executes the behavioral tests, so neither CI nor hk first runs a
duplicate ordinary suite. Keep only one coverage writer active per target
directory, and preserve the instrumented child fixtures.

Coverage prepares the verified engine archives and uses the supervisor from its
single instrumented workspace build. Its fixtures initialize the engine cache
when needed. Ordinary package tests still prepare a private supervisor snapshot
before they run, so concurrent Cargo builds cannot replace their executable.
Coverage explicitly clears that snapshot opt-in and does not compile an unused
ordinary supervisor first.

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

## Bundled engine build inputs

Every Kuru executable contains its target's pinned full-Dolt archive and license
notices. Installed applications extract this engine locally, including on a
first offline launch. The archive is a build input; memory provisioning never
downloads it at runtime.

`packages/kuru-memory/support/dolt-assets.json` owns the exact version, target,
upstream URL, sizes and digests. The memory package's `bundle:prepare` task invokes
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

The cached native CI job selects `${{ runner.temp }}/kuru-bundles` for all its
preparation and build steps. Private bundle directories must be created by the
current runner; restoring them inside a Cargo target archive can change their
permissions. Keep them outside shared build-output caches and retain the private
directory checks when configuring native test runners.

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

## Change workflow

```sh
mise run cospec -- new feat example-change
mise run cospec -- instructions proposal --change example-change
# Author the indicated artifacts and acceptance ledger.
mise run cospec -- validate example-change --strict
mise run cospec -- apply example-change
# Implement, test, and record actual evidence.
mise run format:code ::: lint:rust ::: typecheck ::: coverage ::: lint:tooling ::: cospec:validate ::: cospec:managed:check ::: docs:check
mise run cospec -- archive example-change
```

Use the change type that matches the intended conventional commit. The apply
gate must succeed before implementation. Archive is performed through cospec
once tasks and evidence are complete, before the final change commit. Generated
schemas and harness instructions are updated by cospec, not edited manually.
After archival, replace any generated purpose placeholders in new durable specs
with their capability purpose and rerun `mise run cospec:validate`.

hk validates format/tooling/specs before commits, separate concurrent quality
steps before pushes, and conventional commit titles. Hooks are installed by mise's postinstall and
`mise run setup`. Fix failed checks instead of bypassing hooks.

Git hooks export repository-selection variables, so a subprocess working directory
alone does not isolate another checkout. Delivery's rooted command constructor
clears that inherited context for Git and tools that invoke Git, then applies
intentional overrides such as the temporary release index. Keep new repository
subprocesses on that path and exercise foreign hook environments through child
processes with temporary repositories. See [Git's hook documentation](https://git-scm.com/docs/githooks).

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

## Dependency and release updates

Change workspace dependency pins centrally and regenerate Cargo.lock. Change
tool pins with the matching five-platform lock refresh:

```sh
mise lock --platform linux-x64,linux-arm64,macos-x64,macos-arm64,windows-x64
mise -C apps/kuru-docs lock --platform linux-x64,linux-arm64,macos-x64,macos-arm64,windows-x64
mise -C packages/kuru-delivery lock --platform linux-x64,linux-arm64,macos-x64,macos-arm64,windows-x64
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
See mise's [lockfile provenance contract](https://mise.jdx.dev/dev-tools/mise-lock.html#provenance-and-security)
and [task tool configuration](https://mise.jdx.dev/tasks/task-configuration.html#tools).

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
and A2A fixtures exercise their own wire protocols. Use temporary project,
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
