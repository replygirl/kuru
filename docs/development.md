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
and arm64 and macOS arm64 binaries; it does not publish an Intel macOS binary.
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
tool pins with the matching four-platform lock refresh:

```sh
mise lock --platform linux-x64,linux-arm64,macos-x64,macos-arm64
mise -C apps/kuru-docs lock --platform linux-x64,linux-arm64,macos-x64,macos-arm64
mise -C packages/kuru-delivery lock --platform linux-x64,linux-arm64,macos-x64,macos-arm64
```

Mise records available provenance for every platform, but normally verifies
only the current platform's artifact. Before committing an updated Communiqué
lock entry, verify the other supported archives as well. These commands download
and verify artifact bytes; they neither install nor execute foreign binaries:

```sh
MISE_OS=linux MISE_ARCH=x86_64 mise -C packages/kuru-delivery lock github:jdx/communique --platform linux-x64
MISE_OS=linux MISE_ARCH=aarch64 mise -C packages/kuru-delivery lock github:jdx/communique --platform linux-arm64
MISE_OS=macos MISE_ARCH=aarch64 mise -C packages/kuru-delivery lock github:jdx/communique --platform macos-arm64
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

Memory tests use actual full Dolt. `packages/kuru-memory` owns the verified engine
prefetch, supervisor fixture and integration checks. Runtime/TUI test tasks depend
on those fixtures; missing engines fail tests. Run
`mise run //packages/kuru-memory:prefetch` to populate the test cache ahead of an
offline run. Test helpers use isolated stores and a shared verified temporary
cache, never the user's memory. The test runner uses two threads and limits
simultaneous temporary servers. Do not replace these fixtures with SQLite or
exclude memory modules from coverage.

The demo provider allows offline process smoke tests. Protocol tests start
local fake app-server, HTTP, MCP or A2A peers and exercise actual wire framing.
Use temporary project roots and memory stores. Never inspect or copy the user's
Codex credential file to construct test fixtures; supported auth status and
model discovery commands are the intended read-only probes.
