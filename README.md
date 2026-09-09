# Kuru

A Rust terminal chat harness whose native architecture is a pool of persistent
peers. Each part has private memory, can address any other part, and can form a
temporary relationship with its own speaking identity and durable history.
Framework modes change the organization of the pool: IFS, polyvagal, Freudian,
and Jungian. A mechanical scheduler enforces budgets; no model supervises the
other models.

Kuru treats psychological frameworks as computational metaphors. It does not
claim consciousness, reproduce a human nervous system, or provide therapy.

## Install from this checkout

Kuru has not been published to a public release repository. These source
installation paths work from a checkout today. Building requires Rust 1.98.1,
a C compiler for bundled SQLite, and standard platform build tools.

Direct installation with an existing Rust toolchain:

```sh
cd /path/to/kuru
cargo install --path apps/tui --locked
kuru --version
```

This installs to Cargo's binary directory, normally `~/.cargo/bin`. To choose
`~/.local/bin` or another destination, use the source installer:

```sh
KURU_INSTALL_DIR="$HOME/.local/bin" bash scripts/install.sh --source
```

With [mise](https://mise.jdx.dev/), from the checkout:

```sh
mise trust
mise install
mise run setup
mise run install
```

The mise path installs the pinned toolchain and development tools before
building the same executable. Add your installation directory to `PATH`.
See [installation and updates](docs/install.md) for release archives and updates.

## Start a conversation

```sh
kuru --provider demo run "Help me think through a difficult design decision."
kuru --provider demo
```

The demo provider exercises the pool locally without credentials. For live
OpenAI models, install the verified current Codex release (`npm install -g
@openai/codex@0.153.4`, or the pinned tool through `mise install`), authenticate through its supported login flow,
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
changes can be reversed. Sessions persist locally in SQLite. Jungian collective
memory is scoped to the project in this first version.

Read [usage and terminal controls](docs/usage.md), [architecture](docs/architecture.md), [protocols and tools](docs/protocols.md),
and [development](docs/development.md) for boundaries and extension points.

## Repository

| Path | Responsibility |
| --- | --- |
| `apps/tui` | Terminal UI and `kuru` executable |
| `packages/kuru-core` | Frameworks, configuration and SQLite memory |
| `packages/kuru-connectors` | Providers, tools, MCP and outbound A2A |
| `packages/kuru-runtime` | Actor pool, peer routing, relationships and dreaming |
| `openspec` | cospec change workflow and capability specifications |
| `scripts` | Installation, releases and repository checks |

`mise run check` runs format, Clippy, behavioral tests, installer tests, workflow
validation, cospec checks and a 90% workspace line-coverage gate. Releases are
built by the tagged release workflow; nothing is published by local setup.
The [verification record](docs/verification.md) includes measured coverage,
live OpenAI checks and installation results.

MIT licensed.
