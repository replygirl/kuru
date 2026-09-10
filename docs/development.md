# Development

Install pinned tooling with `mise install`, then run `mise run setup`. Rust 1.98.1
is declared in both mise and rust-toolchain.toml. Cargo.lock pins runtime
transitives. The [dependency audit](dependencies.md) records latest stable
versions and the exact upstream constraints on transitive updates. mise.lock contains platform-specific tool URLs and checksums.
Cospec is a standalone executable with embedded OpenSpec. Its validate/apply
JSON and managed-file checks run without a project OpenSpec dependency. The
pinned 0.7.0 release bundles two OpenSpec entrypoint calls into one file, causing
duplicate execution. The cospec mise task scopes a small
[compatibility preload](../packages/kuru-delivery/support/cospec-preload.cjs) to
that exact bundle hash using cospec's own runtime. It preserves command arguments
and the original gate; native integration tests prove clear, hard-blocked,
soft-blocked and missing-artifact outcomes. Remove the preload after an upstream
release fixes vendoring and passes those standalone tests.

The architecture follows this order: apps/ and packages/ ownership, mise
monorepo tasks, Rust, then other tools. Every app/package owns a mise.toml;
root aliases use `//path:task` addresses. For example,
`mise run //apps/kuru-tui:test` works from any directory, and `mise run test`
inside that app runs its tests. Cargo's workspace shares dependency resolution
and a single coverage report. It does not replace mise task ownership.

Installation, packaging, release orchestration and repository checks live in
the Rust package `packages/kuru-delivery`. No Python or Bun is needed. The
VitePress docs app owns its Node pin, package.json and npm lockfile under
`apps/kuru-docs`; its mise tasks invoke the installed tools directly.

The delivery package activates Cocogitto and Communiqué only for its tests,
combined coverage and release tasks. Its `setup` task preinstalls those tools
with mise's `--include-task-tools` option. Communiqué 1.3.5 provides Linux x86_64
and arm64, macOS arm64 and Windows x86_64 binaries; it does not publish an Intel macOS binary.
Run the full maintainer gate on one of those supported platforms. Building,
installing and packaging Kuru on Intel macOS uses the Rust tasks and does not
require Communiqué or maintainer setup.

CI also builds and packages the native executable on Intel macOS and Linux
arm64, and requires those jobs alongside the full gates. Archive-only jobs
install locked Rust with automatic task-tool installation disabled. They set
`MISE_NO_HOOKS=1` to omit mise's repository-setup postinstall hook: hk 1.58.1
does not publish an Intel macOS binary, and these jobs do not create Git commits.
Local Git hooks, maintainer setup and the full validation jobs retain hk.

## Commands

| Command | What it checks or runs |
| --- | --- |
| `mise run build` | Locked debug workspace build |
| `mise run build:release` | Optimized release build |
| `mise run run -- --provider demo` | Interactive offline harness |
| `mise run format:fix` | Rust and TOML formatting |
| `mise run lint` | All-target Clippy with warnings as errors |
| `mise run test` | Workspace behavioral and protocol tests |
| `mise run coverage` | Workspace LLVM line coverage, minimum 90% |
| `mise run test:install` | Native archive and real Bash bootstrap installation, rejection and interruption tests |
| `mise run lint:tooling` | Shell, GitHub Actions and metadata validation |
| `mise run docs:dev` | Local VitePress server |
| `mise run docs:check` | Production docs, local links, anchors and public content boundary |
| `mise run release:version` | Conventional-commit version calculation without publication |
| `mise run cospec:validate` | Strict validation of changes and durable specs |
| `mise run cospec:managed:check` | Generated cospec-file drift |
| `mise run check` | Complete required gate |

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

The default cache is `target/kuru-bundles` at the workspace root. Each archive is
named `<archive_sha256>.archive`. `KURU_DOLT_BUNDLE_DIR` selects another absolute
directory for both preparation and compilation. Valid files are reverified and
reused; corrupt or unsafe entries fail without replacement. This build cache is
separate from the installed application's extracted `memory.cache_dir`.

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
`bash scripts/install.sh --source` for source installation: it prepares Rust and
the bundled input through mise, then installs the complete executable. See
[installation](install.md#build-from-source) for requirements and destinations.

## Change workflow

```sh
mise run cospec -- new feat example-change
mise run cospec -- instructions proposal --change example-change
# Author the indicated artifacts and acceptance ledger.
mise run cospec -- validate example-change --strict
mise run cospec -- apply example-change
# Implement, test, and record actual evidence.
mise run check
mise run cospec -- archive example-change
```

Use the change type that matches the intended conventional commit. The apply
gate must succeed before implementation. Archive is performed through cospec
once tasks and evidence are complete, before the final change commit. Generated
schemas and harness instructions are updated by cospec, not edited manually.
After archival, replace any generated purpose placeholders in new durable specs
with their capability purpose and rerun `mise run cospec:validate`.

hk validates format/tooling/specs before commits, the full gate before pushes,
and conventional commit titles. Hooks are installed by mise's postinstall and
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
the supervisor fixture and integration checks. Runtime/TUI test tasks depend on
those fixtures. `mise run //packages/kuru-memory:prefetch` extracts and verifies
the embedded engine into the shared test cache. Its build dependency prepares
the archive as described above, downloading it only when needed and permitted.
Cold-cache tests
also exercise first offline extraction, so a populated cache is not a runtime
prerequisite. Test helpers use isolated stores, never the user's memory.
The test runner uses two threads and limits
simultaneous temporary servers. Do not replace these fixtures with SQLite or
exclude memory modules from coverage.

The demo provider allows offline process smoke tests. Protocol tests start
local fake app-server, HTTP, MCP or A2A peers and exercise actual wire framing.
Use temporary project roots and memory stores. Never inspect or copy the user's
Codex credential file to construct test fixtures; supported auth status and
model discovery commands are the intended read-only probes.
