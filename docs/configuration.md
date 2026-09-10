# Configuration

Kuru uses typed TOML. Layers merge in this order: built-in defaults, user defaults,
ancestor `.kuru/config.toml` files from outermost to innermost directory, remembered
interactive choices for the project, an explicit `--config` file, and CLI flags.
Later values win; tables merge recursively and arrays
replace. Unknown keys, invalid types, unsupported provider names and invalid
bounds fail with context. Each configuration file is bounded to 256 KiB and the
combined input to 1 MiB.

Use `kuru config` and `kuru --help` to inspect effective options and CLI overrides.
`kuru config` redacts MCP environment values. User defaults are read from
`$XDG_CONFIG_HOME/kuru/config.toml` or `~/.config/kuru/config.toml`; `--config`
selects the final local layer. CLI flags take precedence over file values.
Ancestor `AGENTS.md` files provide project instructions, ordered so local
instructions have precedence. Kuru does not automatically follow arbitrary
links in instruction files; repositories can put their applicable instructions
in AGENTS.md itself.

Mode, model and effort choices made in the terminal with F2/F3/F4 or the matching
slash commands are saved immediately. Relaunching from the same canonical directory
and data store restores those choices in a new conversation. A symlink to that
directory shares its choices; another directory has its own. Kuru keeps these
preferences in its private Dolt store and does not edit project configuration.
Persistence failures are reported before a new choice becomes active.

Models and efforts are remembered together for each provider. `/model` selects the
new model's advertised default effort, and `/effort default` explicitly clears a
previously configured effort. An explicit `--model` or `--config` model that differs
from the remembered model uses the invocation's file effort or provider default;
it does not inherit the previous model's remembered effort. Explicit `--effort`
still takes precedence. Flags and `--config` overrides apply to that invocation
without replacing saved choices. Interactive changes during that invocation are
saved normally.

`--resume SESSION` restores that conversation's framework, including when a mode
was supplied for the invocation; it does not change the remembered framework for
future fresh conversations. Session history is only resumed when requested.

This example contains the default scalar values:

```toml
mode = "ifs"
provider = "codex"
model = "auto"
max_rounds = 3
max_tool_calls = 12
max_parallel = 4
dream_every = 8
dream_on_exit = true
max_parts = 16
allow_shell = false
allow_write = false
codex_command = "codex"
api_base = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"
```

`mode` is `ifs`, `polyvagal`, `freudian` or `jungian`. `provider` is `codex`,
`responses` or `demo`. The optional `effort` field is a provider-advertised
string, such as `effort = "high"`. Models and effort levels vary by account and
provider; consult `kuru models` instead of relying on a hardcoded list.
`model = "auto"` uses provider selection. Kuru preserves newly advertised effort
strings. The API key itself never belongs in configuration.

The public Responses `/models` catalog does not advertise a default chat model
or supported reasoning efforts. With `provider = "responses"`, set an explicit
`model` (or `--model`) and, when needed, `effort`. Kuru passes these through and
surfaces provider validation errors; it does not select an arbitrary audio or
embedding model from that catalog.

`max_rounds` is 1–64, `max_tool_calls` 1–1024 and `max_parallel` 1–64.
`max_parts` must fit the built-in topology and cannot exceed 128.
`dream_every = 0` disables periodic dreaming; explicit and session-end dreaming
remain separate. Set `dream_on_exit = false` to disable exit dreaming.

## MCP servers

A server has exactly one transport: `command` for stdio or `url` for HTTP.
Aliases namespace the tools exposed to parts. At most 64 servers are accepted.

```toml
[mcp.local_service]
command = "/absolute/path/to/mcp-server"
args = ["--stdio"]

[mcp.remote_service]
url = "https://example.com/mcp"
```

Stdio servers may have an `env` table; avoid storing credentials in shared
configuration. HTTP entries cannot contain process arguments or an environment
table. Enabling an MCP server means granting the harness access to that server's
tools; Kuru's built-in `allow_write` and `allow_shell` only govern its own tools.

## External agents

```toml
[external_agents]
research_peer = "https://example.com/a2a"
```

At most 64 endpoints are accepted. Use explicit trusted endpoints; credentials
and fragments in URLs are rejected. See [protocols](protocols.md) for the supported
A2A subset and local ingress controls.

## Storage and authority

Session and part histories are local durable data. The default data directory is
`$XDG_DATA_HOME/kuru` or `~/.local/share/kuru`. Each canonical project has a Dolt
database under `memory/<project-hash>/`.
`--data-dir` or `KURU_DATA_DIR` chooses a separate storage directory. An OS
writer lock prevents competing Kuru processes from overwriting the same
project topology; session listing remains available without a writer lock. Restrict access to the user
data directory as you would a chat transcript. They are not included in source
control and should never be exposed as a tool root.

Kuru includes its pinned full-Dolt engine and licenses in the executable. First
memory use verifies and extracts them locally; later runs reuse the verified
cache. These optional settings control the extracted runtime:

```toml
[memory]
offline = false
startup_timeout_secs = 30
# cache_dir = "/absolute/path/to/dolt-cache"
# dolt_binary = "/absolute/path/to/dolt"
```

The default engine cache is `tools/dolt` inside the data directory. A fresh cache
works offline without a separate engine installation. `offline` remains accepted
for compatibility; bundled engine provisioning never uses HTTP, and this setting
does not disable provider network calls. `dolt_binary` is an optional development
override that must report the supported exact version. Corrupt existing caches
fail without automatic repair. The startup timeout is 1–300 seconds. See
[memory storage](memory.md) for migration, revision inspection and backups, or
[development](development.md#bundled-engine-build-inputs) for build-input settings.

Writes and shell execution require opt-in through config or the corresponding
CLI flags. Enabling shell permits subprocess activity with your account's
permissions, including network access; the working directory does not constrain
what a subprocess can access. Built-in file tools separately enforce canonical
root containment, including symlinks, and protect instructions, configuration
and state paths.
