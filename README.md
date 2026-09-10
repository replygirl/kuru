# Kuru

A Rust terminal chat harness whose native architecture is a pool of persistent
peers. Each part has private memory, can address any other part, and can form a
temporary relationship with its own speaking identity and durable history.
Framework modes change the organization of the pool: IFS, polyvagal, Freudian,
and Jungian. A mechanical scheduler enforces budgets; no model supervises the
other models.

Kuru treats psychological frameworks as computational metaphors. It does not
claim consciousness, reproduce a human nervous system, or provide therapy.

[Documentation](https://replygirl.github.io/kuru/) · [Development](docs/development.md)

## Install

### mise

With [mise](https://mise.jdx.dev/getting-started.html):

```sh
mise use -g github:replygirl/kuru
kuru --version
```

This installs and activates the native executable. To select an exact release,
use `mise use -g github:replygirl/kuru@VERSION`. Replace `VERSION` with a full
`major.minor.patch` version from [releases](https://github.com/replygirl/kuru/releases)
that includes an archive for your platform. Exact versions also bypass mise's
release-age cooldown for newly published releases.

### macOS and Linux

Install the latest native release into `~/.local/bin`:

```sh
curl -fsSL https://raw.githubusercontent.com/replygirl/kuru/main/packages/kuru-delivery/support/install.sh | bash
export PATH="$HOME/.local/bin:$PATH"
kuru --version
```

The installer selects your platform, verifies the archive checksum, and replaces
the executable atomically.

### Windows

In Windows PowerShell 5.1:

```powershell
irm https://raw.githubusercontent.com/replygirl/kuru/main/packages/kuru-delivery/support/install.ps1 | iex
$env:PATH = "$env:LOCALAPPDATA\Programs\kuru\bin;$env:PATH"
kuru --version
```

Add that installation directory to your user `PATH` for future terminals. Binary
installation needs no separately installed compiler or MSVC redistributable.
The native targets are macOS/Linux arm64 and x86-64, and Windows x86-64;
see [installation and updates](docs/install.md) for platform requirements,
version selection, destinations and offline installation.

Kuru includes its native Dolt engine and licenses. First memory use extracts them
locally, including offline. See [memory storage](docs/memory.md) for migration
and revision history.

### From source

Building requires mise, a C compiler for the legacy SQLite importer, and standard
platform build tools. The source installer prepares the pinned Rust toolchain
and bundled engine input:

```sh
git clone https://github.com/replygirl/kuru.git
cd kuru
bash scripts/install.sh --source
```

On Windows, install the Visual Studio C++ Build Tools and Windows SDK, then run
`& .\scripts\install.ps1 -Source` from the checkout. The source installers prepare
their build inputs through package-owned mise tasks.

The executable installs into `~/.local/bin` on macOS/Linux or
`$env:LOCALAPPDATA\Programs\kuru\bin` on Windows. To choose another destination
on macOS/Linux:

```sh
KURU_INSTALL_DIR="$HOME/.local/bin" bash scripts/install.sh --source
```

On Windows, pass `-InstallDir C:\Tools\kuru\bin` to the source entrypoint.

For the repository's pinned maintainer toolchain and mise tasks, see
[development](docs/development.md).

## Start a conversation

```sh
kuru --provider demo run "Help me think through a difficult design decision."
kuru --provider demo
```

The demo provider exercises the pool locally without credentials. For live
OpenAI models, install the repository's pinned Codex version (`npm install -g
@openai/codex@0.153.4`, or the pinned tool through `mise install` from a maintainer
checkout), authenticate through its supported login flow,
and start Kuru with the default `codex` provider. Model and reasoning-effort
choices are discovered from your provider at runtime. API-key users can select
the `responses` provider with `OPENAI_API_KEY` set in their environment.

```sh
kuru login
kuru models
kuru
```

For API-key authentication, use an explicit model:

```sh
kuru --provider responses --model gpt-6-astra --effort low
```

Use `kuru --help` for current CLI flags and commands. Configuration can live at
user, ancestor project-directory, and explicit local scope. Writes and shell
execution are disabled by default. [Configuration](docs/configuration.md)
describes permissions, budgets, models, MCP servers and external agents.

## How the pool works

IFS starts with Self, managers, firefighters and exiles; the other modes supply
their own roles. Parts report modeled state and propose peer messages or
relationships. Protection, polarization and alliance groups contain two to four
parts and can become the user-facing identity. Their memories are separate
from both the shared conversation and each member's private history.

Dreaming gathers bounded proposals that can add or retire parts. Retired
histories remain stored, each framework role remains represented, and topology
changes can be reversed. Sessions persist locally in versioned Dolt databases. Jungian collective
memory is scoped to the project in this first version.

Read [usage and terminal controls](docs/usage.md), [architecture](docs/architecture.md), [protocols and tools](docs/protocols.md),
and [development](docs/development.md) for boundaries and extension points.

## Repository

| Path | Responsibility |
| --- | --- |
| `apps/kuru-tui` | Terminal UI and `kuru` executable |
| `apps/kuru-docs` | VitePress docs and its local Node/npm dependencies |
| `packages/kuru-core` | Frameworks, configuration and shared contracts |
| `packages/kuru-memory` | Managed Dolt and private versioned memory |
| `packages/kuru-platform` | Checked filesystems and Windows process/IPC primitives |
| `packages/kuru-archive` | Bounded archive codecs shared by delivery and memory |
| `packages/kuru-connectors` | Providers, tools, MCP and outbound A2A |
| `packages/kuru-runtime` | Actor pool, peer routing, relationships and dreaming |
| `packages/kuru-delivery` | Native installation, release and repository tooling |
| `openspec` | cospec change workflow and capability specifications |
| `scripts` | Small installation and commit-hook shell entrypoints |

CI and hk run format, Clippy, typecheck, tooling, cospec, docs and behavioral
coverage as independent checks. `mise run coverage` runs the test suite with a
90% workspace line-coverage gate; `mise run check` is an optional local aggregate.
Each app/package owns its mise tasks; root commands are aliases and
aggregations. Releases use a manual workflow with conventional-commit versioning
and Communiqué notes; nothing is published by local setup.
The [verification record](docs/verification.md) includes measured coverage,
live OpenAI checks and installation results.

Development follows [aligned-team/cospec](https://github.com/aligned-team/cospec)
as its reference standard. See [contributing](CONTRIBUTING.md) for the branch,
review and cospec workflow, and [security](SECURITY.md) for private reporting.

MIT licensed.
