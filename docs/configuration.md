# Configuration

Kuru uses typed TOML. Layers merge in this order: built-in defaults, user defaults,
ancestor `.kuru/config.toml` files from outermost to innermost directory, remembered
interactive choices for the project, an explicit `--config` file, and CLI flags.
Later values win; tables merge recursively and arrays
replace. Unknown keys, invalid types, unsupported provider names and invalid
bounds fail with context. Each configuration file is bounded to 256 KiB and the
combined input to 1 MiB.

Use `kuru config` and `kuru --help` to inspect configured options and CLI overrides.
`kuru config` redacts MCP environment values and deliberately omits saved project
mode, model and effort preferences so inspection never starts or provisions
memory. It prints an omission notice on stderr. User defaults are read from
`$XDG_CONFIG_HOME/kuru/config.toml` when set. Otherwise, Kuru uses
`~/.config/kuru/config.toml` on macOS/Linux or
`$env:APPDATA\kuru\config.toml` on Windows. Windows also falls back to
`$env:USERPROFILE\AppData\Roaming` when `APPDATA` is unset. `--config` selects the
final local layer. CLI flags take precedence over file values.

When a command opens memory, fixed progress messages appear on standard error
while Kuru acquires private ownership, verifies or extracts the bundled runtime,
and opens the database. They describe work in progress, not an estimate or a
successful open; JSON and other command results remain on standard output.
Every ancestor `AGENTS.md`, including the project root, can provide automatic
project instructions. Kuru captures the present files from outermost to most
local before workspace review, and local instructions have precedence. The
reviewed snapshot owns the exact bytes later placed in prompts, so Kuru does not
reopen those paths after approval. Kuru does not automatically follow arbitrary
links in instruction files; repositories can put their applicable instructions
in `AGENTS.md` itself.

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
api_base = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"
```

`mode` is `ifs`, `polyvagal`, `freudian` or `jungian`. `provider` is `codex`,
`responses` or `demo`. The optional `effort` field is a provider-advertised
string, such as `effort = "high"`. Models and effort levels vary by account and
provider; consult `kuru models` instead of relying on a hardcoded list.
`model = "auto"` uses provider selection. Kuru preserves newly advertised effort
strings. The API key itself never belongs in configuration.

## Workspace trust

Kuru reviews effective process, mutation, endpoint and prompt authority supplied
by automatically discovered ancestor `.kuru/config.toml` and `AGENTS.md` files
before activating it. This includes an `AGENTS.md` at the project root and any
automatic source under your home directory; location alone does not make a file
an explicit caller input. Kuru has no separate global-instruction source.
The trust subject is the exact canonical `-C` directory and its current native
directory identity. Approval does not inherit to parent or child directories.
User defaults, an explicit `--config` file and CLI flags are deliberate caller
inputs; an effective value supplied by one of those layers does not require
workspace approval.

```sh
kuru -C /path/to/project trust status
kuru -C /path/to/project trust approve
kuru -C /path/to/project trust approve --yes
kuru -C /path/to/project trust revoke
kuru -C /path/to/project --trust-workspace-once tools
```

`trust status` displays the normalized root, automatic ancestor sources, safe
claim descriptions and whether the complete current manifest matches its stored
approval. The project-instructions claim lists its bounded source labels in
outermost-to-most-local order and never prints instruction contents. `trust
approve` reviews and stores that complete manifest; `--yes` is the explicit
noninteractive form. Any automatic authority addition, removal or value change,
or any added, removed, reordered, replaced or changed automatic instruction
source, invalidates the whole stored approval. A change only to mode, model,
effort, budgets, dreaming, memory offline state or timeouts leaves it valid.
`trust revoke` removes the exact root's record without confirmation. Status and
an absent revoke create no trust directory, lock or record.

`--trust-workspace-once` authorizes only the current command's applicable claims
and writes no approval. A matching complete stored approval may satisfy a command
that uses only part of the manifest; individual stored claim digests are audit
data and never act as partial grants. Without an approval or the one-time flag,
noninteractive commands fail with a bounded review and the remedy. Interactive
TUI startup asks before entering the alternate screen and offers continue once,
approve the complete configuration, or cancel.

The activation sets are command-specific:

| Commands | Automatic ancestor authority checked before activation |
| --- | --- |
| `login`, `logout`, `config`, `trust ...`, `update` | None; login/logout use the fixed ChatGPT route, config omits saved preferences, and revoke does not parse current workspace configuration |
| `auth` | Active Responses route; under another provider its API-key availability is not checked |
| `sessions`, `memory ...`, `undo-dream` | Configured memory executable and cache paths |
| `models` | Configured memory paths used while loading saved selections, plus an active Responses route |
| `tool`, `tools` | Configured memory paths used while loading saved selections, built-in write/shell grants, and stdio/HTTP MCP configuration |
| `run`, `dream`, `serve`, interactive TUI | All applicable project-instruction, memory, provider, write, shell, MCP and external-agent claims |

The other rows do not construct peer prompts, so they do not consume the
project-instructions claim. Reading a snapshot for `config` or trust inspection
does not inject its instruction bytes or activate a provider.

Review output and configuration errors escape and bound source labels and hide
MCP arguments and environment values, URL queries and credential values. Approval
records live in checked private files under `<data-dir>/trust/workspaces`, outside
Dolt and the tool root, and contain digests rather than configuration values.

Workspace trust authorizes configuration; it does not restrict an approved
process. Shell and stdio MCP processes retain your account's process authority.
Kuru retains the reviewed workspace directory and detects an observed pathname
replacement before configured cwd-based launches. On Unix, a replacement can
still race between that check and child startup; this is not an atomic cwd binding
or an OS sandbox.

The public Responses `/models` catalog does not advertise a default chat model
or supported reasoning efforts. With `provider = "responses"`, set an explicit
`model` (or `--model`) and, when needed, `effort`. Kuru passes these through and
surfaces provider validation errors; it does not select an arbitrary audio or
embedding model from that catalog.

`max_rounds` is 1–64, `max_tool_calls` 1–1024 and `max_parallel` 1–64.
`max_parts` must fit the built-in topology and cannot exceed 128.
`dream_every = 0` disables periodic dreaming; explicit and session-end dreaming
remain separate. Set `dream_on_exit = false` to disable exit dreaming.

## Authentication

The default `codex` provider uses ChatGPT subscription authentication and direct
HTTP requests. Run `kuru login` for browser sign-in, `kuru login --no-browser`
to open the printed URL yourself, or `kuru login --device` for device
authorization. No Codex executable or app-server is required.

Kuru keeps its credentials in the private `auth/openai` directory under the
[data directory](#storage-and-authority). Use the same `--data-dir` or
`KURU_DATA_DIR` selection for login and subsequent commands. The auth store
must be outside the project tool root. Kuru never imports another application's
credential store, and tokens do not belong in TOML or project files.

`kuru auth` prints redacted local status without creating credentials or opening
project memory. It does not prove that a live model request will succeed.
`kuru logout` clears Kuru's stored ChatGPT credentials; it does not change an
API key supplied by the environment. Kuru refreshes its own session when needed;
if refresh fails or its outcome is uncertain, follow the error's sign-in guidance.

The `responses` provider uses the API key from `api_key_env`, which defaults to
`OPENAI_API_KEY`, and sends requests to `api_base`. These settings apply only
to API-key requests. ChatGPT credentials use the fixed subscription service
and are never sent to `api_base`. Provider selection is explicit: a failed
ChatGPT login or request does not switch to API-key access.

The former `codex_command` option has been removed. Delete it from existing
configuration and run `kuru login` to establish Kuru's own session. Existing
`provider = "codex"` selections remain valid.

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
`$XDG_DATA_HOME/kuru` when set, otherwise `~/.local/share/kuru` on macOS/Linux or
`$env:LOCALAPPDATA\kuru` on Windows. If `LOCALAPPDATA` is unset, Windows falls
back to `$env:USERPROFILE\AppData\Local\kuru`. Each canonical project has a Dolt
database under `memory/<project-hash>/`.
`--data-dir` or `KURU_DATA_DIR` chooses a separate storage directory. Either may
be relative; Kuru resolves relative values from the invocation directory. On
Windows, memory and engine caches require local volumes with persistent ACLs;
UNC shares and device paths are not supported state locations. An OS writer lock
prevents competing Kuru processes from overwriting the same project topology;
session listing remains available without a writer lock. Restrict access to the
user data directory as you would a chat transcript. They are not included in
source control and should never be exposed as a tool root.

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
does not disable provider network calls. `cache_dir` and `dolt_binary` must be
absolute native paths; Kuru does not resolve either relative to a configuration
file. `dolt_binary` is an optional development override that must report the
supported exact version. Corrupt existing caches fail without automatic repair.
If activating a verified engine fails, the error reports the retained private
staging directory for inspection; Kuru does not automatically retry that move.
The startup timeout is 1–300 seconds. See [memory storage](memory.md) for
migration, revision inspection and backups, or
[development](development.md#bundled-engine-build-inputs) for build-input settings.

Writes and shell execution require opt-in through config or the corresponding
CLI flags. Enabling shell permits subprocess activity with your account's
permissions, including network access; the working directory does not constrain
what a subprocess can access. Built-in file tools separately enforce canonical
root containment, including symlinks, and protect instructions, configuration
and state paths.
