# Configuration

Kuru uses typed TOML. Run `kuru config` to inspect configured values and CLI overrides. MCP environment values are redacted, and saved project mode, model, and effort preferences are omitted so this command never opens or provisions memory. The stderr notice identifies that omission. Unknown keys, unsupported providers, invalid types, and invalid bounds produce an error rather than being silently ignored.

## Precedence

Later layers take precedence:

1. Built-in defaults.
2. User defaults at `$XDG_CONFIG_HOME/kuru/config.toml` when set; otherwise `~/.config/kuru/config.toml` on macOS/Linux or `$env:APPDATA\kuru\config.toml` on Windows.
3. Ancestor `.kuru/config.toml` files, from outermost to nearest directory.
4. Remembered interactive choices for the project.
5. An explicit `--config PATH` file.
6. Command-line flags.

Tables merge recursively; arrays replace earlier arrays. Each configuration file is limited to 256 KiB, and combined input to 1 MiB.

[Configuration Schema v1](/configuration.v1.schema.json) describes the
JSON-equivalent structure for editor and tooling support. Kuru's TOML parser and
native semantic validation remain authoritative, especially for URL and
cross-field restrictions. Unknown keys are rejected: a configuration that needs
a new key requires a newer Kuru version and never silently changes authority.

Ancestor `AGENTS.md` files provide project instructions, with nearer files taking precedence. Put applicable instructions in `AGENTS.md` itself; linked files are not automatically followed.

## Workspace trust

Kuru reviews effective process, mutation, executable, credential-route, and endpoint authority supplied by automatic ancestor `.kuru/config.toml` files before activation. The trust subject is the exact canonical `-C` directory and its current native identity; approval does not cover a parent, child, or replacement directory. User defaults, explicit `--config`, and CLI flags are deliberate inputs and authorize their own effective values.

```sh
kuru -C /path/to/project trust status
kuru -C /path/to/project trust approve
kuru -C /path/to/project trust approve --yes
kuru -C /path/to/project trust revoke
kuru -C /path/to/project --trust-workspace-once tools
```

Persistent approval always covers the complete current authority manifest. Any automatic authority value added, removed, or changed invalidates that record globally; ordinary mode, model, effort, budget, dreaming, offline-memory, and timeout changes do not. A command may use the applicable subset of a matching complete record. Stored claim digests are audit data, not separate grants.

`--trust-workspace-once` approves only the invoking command's applicable subset and writes nothing. `trust status` and an absent `trust revoke` create no trust state. Noninteractive commands fail promptly unless a complete stored approval matches or the one-time flag was supplied. Before the terminal UI enters its alternate screen, it offers continue once, approve the complete configuration, or cancel.

| Commands                                           | Automatic ancestor authority checked before activation                                                                                 |
| -------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| `login`, `logout`, `config`, `trust ...`, `update` | None. Login/logout use fixed ChatGPT authentication; config omits saved preferences; revoke works even if current config is malformed. |
| `auth`                                             | Active Responses route. Under another provider the API-key route is reported as not checked.                                           |
| `sessions`, `memory ...`, `undo-dream`             | Configured memory executable and cache paths                                                                                           |
| `models`                                           | Configured memory paths used for saved selections and the active Responses route                                                       |
| `tool`, `tools`                                    | Configured memory paths used for saved selections, write/shell grants, and stdio/HTTP MCP configuration                                |
| `run`, `dream`, `serve`, terminal UI               | All applicable memory, provider, write, shell, MCP, and external-agent claims                                                          |

Review text and configuration diagnostics are bounded and escaped. They do not print MCP arguments or environment values, URL queries, credential values, or raw parser excerpts. Private approval records under `<data-dir>/trust/workspaces` contain root/manifest identities and digests, not configuration values, and remain outside Dolt and workspace tools.

Workspace trust is authorization, not confinement. Approved shell and stdio MCP children keep your process authority. Kuru revalidates the retained workspace before a pathname-based child launch, but Unix cannot atomically bind that check to the later child cwd selection. Workspace trust is not an OS sandbox.

## Remembered choices

<kbd>F2</kbd>, <kbd>F3</kbd>, <kbd>F4</kbd> and their matching slash commands save model, effort, and framework choices immediately. They apply to the canonical project directory and selected data store. A symlink to that directory shares the choices; a different directory has its own.

Preferences live in the private Dolt store. A save failure is reported before the new choice becomes active.

Models and efforts are remembered together for each provider. Selecting a model uses its advertised default effort. Selecting `default` in the effort picker, or `/effort default`, clears an explicit effort.

CLI flags and an explicit configuration file override saved preferences for that invocation without replacing them. Interactive changes made during the invocation are saved normally. An explicitly selected different model uses the invocation's file effort or provider default, not another model's remembered effort; `--effort` still wins.

`--resume SESSION_ID` restores the conversation's framework even if a mode was supplied. It leaves the framework preference for future fresh conversations unchanged.

## Defaults

These are the default scalar values:

```toml
mode = "ifs"
provider = "codex"
model = "auto"
# assumed_context_window_tokens = 128000 # optional; omit to use the built-in fallback
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

## Models and providers

| Key                             | Accepted value                                                                       |
| ------------------------------- | ------------------------------------------------------------------------------------ |
| `mode`                          | `ifs`, `polyvagal`, `freudian`, `jungian`                                            |
| `provider`                      | `codex`, `responses`, `demo`                                                         |
| `model`                         | Provider model ID; `auto` permits provider selection where supported                 |
| `effort`                        | Optional provider-advertised string                                                  |
| `assumed_context_window_tokens` | Optional 1–2,000,000 fallback for models with no advertised or pinned context window |
| `api_base`                      | Responses API base URL                                                               |
| `api_key_env`                   | Environment variable containing the API key                                          |

Use `kuru models` to discover current capabilities. Kuru preserves newly advertised effort strings. The Responses catalog does not supply a default chat model, so that provider requires an explicit `model` or `--model`.

An unknown model remains selectable. Kuru resolves its context window from live
route advertisement, then a route-matched offline snapshot, then this optional
fallback setting, and finally a labelled built-in 128,000-token assumption. The
setting does not override a known provider limit. Prices remain absent when not
verified; Codex subscription prices, where shown later, are labelled
API-equivalent estimates rather than billing or quota facts.

The default `codex` provider uses Kuru's own ChatGPT login and direct subscription
requests. The `responses` provider uses an API key; `api_base` and `api_key_env`
apply only to that provider. Setting an API key does not change the selected
provider, and ChatGPT credentials are never sent to the configurable API base.

Provider diagnostics use fixed status and supported-code categories. Kuru does
not show remote error messages, unknown codes, raw parser input, or configured
endpoint queries. Failed provider bodies are read only up to 8 KiB and two
seconds within the existing request budget; a failed or partial stream is not
replayed.

Kuru may retry only an explicit HTTP 429, 500, or 503 rejection before accepting
a response, with at most three provider sends, two delays, and four total
application sends when a subscription credential rotation is included. Missing
or invalid `Retry-After` values use equal jitter in the ranges 250–500
milliseconds and then 500–1000 milliseconds; a valid delta-seconds or canonical
HTTP date can raise a delay, but never beyond 30 seconds per delay, 60 seconds
total, or the operation deadline. A valid value that cannot fit those bounds
prevents a retry instead of being clamped. API
quota/billing failures, ambiguous transport results, and every failure after a
response or stream is accepted remain terminal and are not replayed.

Subscription credential rotation is limited to one logical rotation per
operation. A token refresh may repeat once only after the pinned native HTTP/1
transport proves the first POST was not dispatched. That audited
connection-acquisition class can itself report a timeout, but a timeout or proxy
CONNECT observation alone proves nothing about dispatch. Responses and other
possibly dispatched results are reconciled against Kuru's private credential
record without replay. These bounds do not promise remote
request idempotency and do not add retry behavior to MCP or A2A transports.

An API key belongs in its environment variable, not the TOML file. Kuru keeps
ChatGPT credentials in the private `auth/openai` directory beneath its data
directory. Use the same `--data-dir` or `KURU_DATA_DIR` for login and chat, and
keep this state outside project tool roots. See [authentication](/guide/authentication).

The former `codex_command` setting has been removed. Delete it from existing
TOML, retain `provider = "codex"`, and run `kuru login` to establish Kuru's own
session. No external Codex executable or credential-store import is needed.

## Budgets and dreaming

| Key              | Meaning                       | Bounds                                    |
| ---------------- | ----------------------------- | ----------------------------------------- |
| `max_rounds`     | Peer rounds per turn          | 1–64                                      |
| `max_tool_calls` | Tool-call budget per turn     | 1–1024                                    |
| `max_parallel`   | Concurrent work limit         | 1–64                                      |
| `max_parts`      | Active pool size limit        | Must fit built-in membership; at most 128 |
| `dream_every`    | Turns between periodic dreams | `0` disables the periodic trigger         |
| `dream_on_exit`  | Consolidate at session end    | Boolean                                   |

`--no-dream` disables both automatic triggers for one invocation. Explicit dreams are separate. See [sessions and dreaming](/concepts/sessions).

## Tool permissions

`allow_write` enables built-in file writes and deletion. `allow_shell` enables ordinary shell processes. The corresponding CLI flags enable them for one invocation.

The shell has your process authority; the project working directory does not constrain what it can access. MCP tools retain their own permissions. See [tools and permissions](./tools).

## MCP and external agents

```toml
[mcp.local_service]
command = "/absolute/path/to/mcp-server"
args = ["--stdio"]

[mcp.remote_service]
url = "https://example.com/mcp"

[external_agents]
research_peer = "https://example.com/a2a"
```

MCP servers have exactly one transport: `command` or `url`. Stdio entries may have `args` and `env`; HTTP entries may not. At most 64 MCP servers and 64 external agent endpoints are accepted. Endpoint URLs reject embedded credentials and fragments.

The example endpoints are placeholders. Choose trusted services and keep secrets out of shared configuration. [MCP](./mcp) and [A2A](./a2a) describe supported behavior and limitations.

## Storage and environment

| Variable or option            | Purpose                                                              |
| ----------------------------- | -------------------------------------------------------------------- |
| `XDG_CONFIG_HOME`             | User configuration base directory                                    |
| `XDG_DATA_HOME`               | User data base directory                                             |
| `APPDATA`, `LOCALAPPDATA`     | Windows configuration and data defaults when XDG overrides are unset |
| `USERPROFILE`                 | Windows fallback for `AppData\Roaming` and `AppData\Local`           |
| `KURU_DATA_DIR`, `--data-dir` | Separate Kuru storage directory                                      |
| `KURU_REDUCED_MOTION=1`       | Static TUI ornament; operation indicators remain live                |
| `KURU_A2A_TOKEN`              | Default bearer-token variable for `serve`                            |
| `KURU_INSTALL_DIR`            | Destination for direct or source installation                        |
| `KURU_RELEASE_BASE`           | Version directory for the release installer                          |

When no XDG data directory is set, the default is `~/.local/share/kuru` on macOS/Linux or `$env:LOCALAPPDATA\kuru` on Windows. If Windows application-data variables are unset, `USERPROFILE` supplies `AppData\Roaming` for configuration and `AppData\Local` for data. Each project has a Dolt database under `memory/<project-hash>/`. `--data-dir` and `KURU_DATA_DIR` may be relative and resolve from the invocation directory. Keep state outside tool roots. [Memory](../concepts/memory) describes project scope and access boundaries.

Windows memory and engine caches require local volumes with persistent ACLs. UNC shares and device paths are not supported state locations.

```toml
[memory]
offline = false
startup_timeout_secs = 30
# cache_dir = "/absolute/path/to/dolt-cache"
# dolt_binary = "/absolute/path/to/dolt"
```

Kuru includes its pinned full-Dolt engine and licenses. First memory use extracts them locally into `tools/dolt` inside the data directory, or the configured `cache_dir`; an empty cache works offline. Existing caches are verified, and corrupt entries fail without automatic repair.

`offline` remains accepted for compatibility; bundled engine provisioning never uses HTTP. `cache_dir` and `dolt_binary` must be absolute native paths and are not resolved relative to a configuration file. `dolt_binary` is an optional development override and must report the supported exact version. The startup timeout is 1–300 seconds. Provider network access is independent of these memory settings.

Commands that open memory print at most one fixed standard-error line for each
actual startup stage: private ownership, runtime cache work, runtime checking,
database preparation, database opening, and ready. These lines do not estimate
duration or establish success; the final command result does. They never alter
standard output, including JSON.
