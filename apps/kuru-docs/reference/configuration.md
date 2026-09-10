# Configuration

Kuru uses typed TOML. Run `kuru config` to inspect the effective configuration; MCP environment values are redacted in its output. Unknown keys, unsupported providers, invalid types, and invalid bounds produce an error rather than being silently ignored.

## Precedence

Later layers take precedence:

1. Built-in defaults.
2. User defaults at `$XDG_CONFIG_HOME/kuru/config.toml` when set; otherwise `~/.config/kuru/config.toml` on macOS/Linux or `$env:APPDATA\kuru\config.toml` on Windows.
3. Ancestor `.kuru/config.toml` files, from outermost to nearest directory.
4. Remembered interactive choices for the project.
5. An explicit `--config PATH` file.
6. Command-line flags.

Tables merge recursively; arrays replace earlier arrays. Each configuration file is limited to 256 KiB, and combined input to 1 MiB.

Ancestor `AGENTS.md` files provide project instructions, with nearer files taking precedence. Put applicable instructions in `AGENTS.md` itself; linked files are not automatically followed.

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

## Models and providers

| Key             | Accepted value                                                       |
| --------------- | -------------------------------------------------------------------- |
| `mode`          | `ifs`, `polyvagal`, `freudian`, `jungian`                            |
| `provider`      | `codex`, `responses`, `demo`                                         |
| `model`         | Provider model ID; `auto` permits provider selection where supported |
| `effort`        | Optional provider-advertised string                                  |
| `codex_command` | Codex executable name or path                                        |
| `api_base`      | Responses API base URL                                               |
| `api_key_env`   | Environment variable containing the API key                          |

Use `kuru models` to discover current capabilities. Kuru preserves newly advertised effort strings. The Responses catalog does not supply a default chat model, so that provider requires an explicit `model` or `--model`.

An API key belongs in its environment variable, not the TOML file. See [authentication](/guide/authentication).

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

When no XDG data directory is set, the default is `~/.local/share/kuru` on macOS/Linux or `$env:LOCALAPPDATA\kuru` on Windows. If Windows application-data variables are unset, `USERPROFILE` supplies `AppData\Roaming` for configuration and `AppData\Local` for data. Each project has a Dolt database under `memory/<project-hash>/`. Keep state outside tool roots. [Memory](../concepts/memory) describes project scope and access boundaries.

Windows memory and engine caches require local volumes with persistent ACLs. UNC shares and device paths are not supported state locations.

```toml
[memory]
offline = false
startup_timeout_secs = 30
# cache_dir = "/absolute/path/to/dolt-cache"
# dolt_binary = "/absolute/path/to/dolt"
```

Kuru includes its pinned full-Dolt engine and licenses. First memory use extracts them locally into `tools/dolt` inside the data directory, or the configured `cache_dir`; an empty cache works offline. Existing caches are verified, and corrupt entries fail without automatic repair.

`offline` remains accepted for compatibility; bundled engine provisioning never uses HTTP. `dolt_binary` is an optional development override and must report the supported exact version. The startup timeout is 1–300 seconds. Provider network access is independent of these memory settings.
