# Development

Install pinned tooling with `mise install`, then run `mise run setup`. Rust 1.98.1
is declared in both mise and rust-toolchain.toml. Cargo.lock pins runtime
transitives. The [dependency audit](dependencies.md) records latest stable
versions and the exact upstream constraints on transitive updates. mise.lock contains platform-specific tool URLs and checksums.
Bun's lockfile pins the development-only OpenSpec dependency used by cospec.
This local dependency avoids a JSON-output defect observed in the standalone
cospec embedded OpenSpec bundle; no reference checkout is required.

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
| `mise run test:install` | Real archive/install and rejection tests |
| `mise run lint:tooling` | Shell, GitHub Actions and metadata validation |
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

## Dependency and release updates

Change workspace dependency pins centrally and regenerate Cargo.lock. Change
tool pins with the matching four-platform lock refresh:

```sh
mise lock --platform linux-x64,linux-arm64,macos-x64,macos-arm64
```

Commit the updated lockfile in the same change. CI detects lock drift. Update
user documentation when flags, configuration, role behavior or contracts change.
[Installation and updates](install.md) describes the tagged release workflow.

## Tests without credentials

The demo provider allows offline process smoke tests. Protocol tests start
local fake app-server, HTTP, MCP or A2A peers and exercise actual wire framing.
Use temporary project roots and memory stores. Never inspect or copy the user's
Codex credential file to construct test fixtures; supported auth status and
model discovery commands are the intended read-only probes.
