# Configuration

Kuru uses typed TOML. Layers merge in this order: built-in defaults, externally
provisioned managed defaults, user defaults, ancestor `.kuru/config.toml` files
from outermost to innermost directory, remembered interactive choices for the
project, `.kuru/config.local.toml` at the exact project root, an explicit
`--config` file, repeatable `-c key=value` values, and dedicated CLI flags.
Later values win; tables merge recursively and arrays
replace. Unknown keys, invalid types, unsupported provider names and invalid
bounds fail with context. Each configuration file is bounded to 256 KiB and the
combined input to 1 MiB.

The published `apps/kuru-docs/public/configuration.v1.schema.json` describes the
JSON-equivalent structure for editor and tooling support. Kuru's TOML parser and
native semantic validation remain authoritative, especially for URL and
cross-field restrictions. Unknown keys are rejected: a configuration that needs
a new key requires a newer Kuru version and never silently changes authority.

Use `kuru config` and `kuru --help` to inspect configured options and CLI overrides.
`kuru config` redacts MCP environment values and deliberately omits saved project
mode, model and effort preferences so inspection never starts or provisions
memory. It prints an omission notice on stderr. User defaults are read from
`$XDG_CONFIG_HOME/kuru/config.toml` when set. Otherwise, Kuru uses
`~/.config/kuru/config.toml` on macOS/Linux or
`$env:APPDATA\kuru\config.toml` on Windows. Windows also falls back to
`$env:USERPROFILE\AppData\Roaming` when `APPDATA` is unset. `--config` selects an
explicit local layer. `-c` accepts TOML values such as `-c max_rounds=4` or
`-c memory.offline=true`; unknown keys and incorrect types are errors. Dedicated
CLI flags take final precedence over `-c` and file values.

Kuru automatically reads `.kuru/config.local.toml` only from the canonical `-C`
directory. It must be a checked regular file and, inside a Git worktree, absent
from Git's index. A tracked local file is rejected. If Git cannot establish its
untracked status, use `--config PATH` explicitly; normal operation does not
require Git when this optional file is absent. Local values are user authority
outside the repository trust manifest, but they do not approve remaining
repository-supplied tool, provider or instruction authority.

An administrator or launcher may set `KURU_MANAGED_CONFIG` to an absolute TOML
path outside the workspace. Its `[defaults]` table uses ordinary configuration
keys at the lowest file precedence. Its `[constraints]` table locks supported
configuration values exactly, after all saved choices and overrides. Arrays
such as `permissions` and each named MCP server table lock as a whole;
external-agent endpoints lock by alias and allow other aliases. An empty
managed table locks that table as empty. A conflict
fails before the affected provider or tool activates; Kuru never substitutes a
different value silently. For example:

```toml
[defaults]
max_rounds = 4
[constraints]
allow_shell = false
max_tool_calls = 12
permissions = []
```

The managed document shape is published at
`configuration.v1.schema.json#/$defs/managed`. Native parsing and semantic
validation remain authoritative.

When a command opens memory, fixed progress messages appear on standard error
while Kuru acquires private ownership, verifies or extracts the bundled runtime,
and opens the database. They describe work in progress, not an estimate or a
successful open; JSON and other command results remain on standard output.
Every ancestor directory through the project root can provide `AGENTS.md` and
`CLAUDE.md` instructions. Kuru reads outermost directories first, then the
project root; within one directory it reads `AGENTS.md` before `CLAUDE.md`.
A standalone line such as `@docs/rules.md` imports a relative Markdown file at
that position. The import must stay within the directory tree of its original
top-level instruction file. Checked file opens reject links and path escapes.
Each physical file is included once, so a sibling `CLAUDE.md` containing
`@AGENTS.md` does not duplicate the `AGENTS.md` text. Import directives inside
fenced code remain literal. Missing or invalid imports stop instruction capture
with a bounded source error.

Each file can contribute at most 256 KiB, with 1 MiB of captured instruction
content in total, at most 128 checked source paths and eight import edges. A
source or branch over these caps is omitted whole; Kuru reports the omission
before inference and in the effective prompt while retaining other usable
instructions. It does not silently truncate a source. The reviewed snapshot
owns the exact active bytes and file/directory identities later placed in
prompts, so Kuru does not reopen those paths after approval. During an actor
turn, a checked file tool can also discover instructions in its authorized
path's nested directories. Kuru composes those sources in the same stable
outermost-to-most-local order and reviews the new complete manifest before
exposing the file result or changing it. A denied file path never activates
its instructions; direct `kuru tool` commands do not inject actor instructions.
For grep and glob, review covers only the bounded, individually authorized
candidate files, not the requested search root or denied siblings.

## Skills and custom prompts

Kuru discovers skill metadata in `.agents/skills/NAME/SKILL.md` under the
project and `skills/NAME/SKILL.md` under Kuru's user configuration directory.
Each file starts with YAML `name` and `description` between `---` lines. Names
use lowercase ASCII letters, digits and single interior hyphens and must match
the directory. Startup reads only the checked frontmatter, then lists effective
names and descriptions in the actor prompt. A project skill takes precedence
over a user skill of the same name. The actor's fixed `skill_load` tool can
select one catalog name and optionally one direct `references/FILE.md` beneath
that skill. Selection captures the full body and requested reference before
exposing either. It never runs files under `scripts/`; `allowed-tools` and
other skill prose do not grant file, shell, MCP or network permission.

Project skill metadata is part of the base workspace manifest. Selecting its
body or reference extends the complete path-qualified manifest and uses the
same once/persist/deny review as nested instructions. A denied selection returns
no skill text. A headless selection without an existing matching grant or
`--trust-workspace-once` returns a trust-required result. The reviewed bytes
remain fixed for the current invocation; later launches recapture changed
sources. User-configuration skills are caller inputs outside repository trust
but follow the same checked file and size rules.

Custom terminal prompt commands live at `.kuru/commands/NAME.md` in the project
or `commands/NAME.md` under Kuru's user configuration directory. They use the
same frontmatter fields followed by nonempty Markdown prompt text. Built-ins
win name collisions, then project commands, then user commands. Effective
project command files join the base manifest; unapproved project names do not
enter `/help` or Tab completion. A custom command expands to a normal user
turn with optional literal arguments, without process execution or extra tool
authority.

Discovery checks at most 256 directory entries and keeps at most 64 effective
skills and 64 commands. Skill frontmatter is limited to 8 KiB per file and
64 KiB total; command files are limited to 64 KiB each and 256 KiB total.
Selected skill bodies and direct references are limited to 256 KiB per file,
128 sources and 1 MiB across the active selection. Invalid, linked, escaping
or over-limit sources are omitted from discovery or rejected at selection with
a bounded notice or tool error; Kuru never presents a partial selected body.

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

## Context budget

Kuru estimates each provider request after its native continuation and tool
results have been assembled. The estimate includes instructions, tool schemas,
current input, history and private continuation. For catalogued GPT-5.6 models
on the official OpenAI Responses and ChatGPT subscription routes, Kuru counts
the final serialized request locally with the embedded `o200k_base` tokenizer
and adds an allowance for provider structure. This remains an estimate: the
provider's token accounting can differ from local JSON tokenization. Unknown
models and custom Responses endpoints retain the historical bytes/2 heuristic
plus a small structural allowance. For a mapped route with native tool
continuation, Kuru tokenizes the visible serialized body and uses that same
byte heuristic for only the saved native output items. This mixed method
retains every pending item in the request. All methods are labelled estimates,
not guaranteed upper bounds. The offline demo retains its earlier bytes/2
estimate.
Sizing makes no extra provider request and does not imply a cache hit. The
status display names the estimate method.

Model windows come from validated route metadata or the pinned catalog. When
neither provides a window, Kuru assumes 128,000 tokens and labels that assumption.
For an unfamiliar model you can configure the fallback window and output reserve:

```toml
assumed_context_window_tokens = 128000
context_output_reserve_tokens = 8192
context_compaction_threshold_percent = 75
context_compaction_output_reserve_tokens = 1024
```

Both optional settings accept 1–2,000,000 tokens. The window setting is a fallback
for missing metadata. Without an output-reserve override, Kuru reserves the
smallest of 8,192 tokens, the model's reported output limit, and a quarter of the
window, with a minimum of one token. A configured reserve is never silently
reduced to fit the window.

The compaction threshold accepts 50–95 percent and defaults to 75. Kuru
compares it with the full policy-admitted request before optional history rows
are omitted for fit. The compaction output reserve accepts 1–2,000,000 tokens
and is separate from the ordinary answer reserve; if it exceeds the effective
model window, compaction refuses before inference. A compaction request is
accounted provider work. It retains every original history row and publishes a
rolling summary only after its exact source revision and cursor still match.

After at most one eligible compaction attempt, older optional cross-session
summaries and history can still be omitted as complete records to fit a request;
the interface reports each actual omission count. The current session's rolling
summary stays mandatory because its raw suffix begins after that summary's exact
cursor. Shared summaries are admitted only for the same actor through the mode's
own-history visibility and memory namespace; another session's raw rows and
producer-private reasoning records are never projected through this path. Stored
memory and conversations remain intact. Current input, required tool receipts and
native continuation are kept together. If mandatory material plus the reserve
cannot fit, the request fails before inference is sent. Transport byte limits
still apply separately.

## Lifecycle hooks

Lifecycle hooks are ordered one-shot commands around admitted actor work. All
five events apply to `run`, `serve`, and interactive conversations. A standalone
`dream` has provider `dream_suggest` tool calls, so only `pre_tool` and
`post_tool` apply there; it has no user turn or selected speaker. They do not run
for configuration or tool inspection, and a direct `kuru tool` remains an
explicit user-authorized one-shot operation. Hooks are not a universal shell
policy or a generic plugin system.

```toml
[hooks]
max_invocations = 256
max_total_ms = 120000
max_annotation_bytes = 262144

[[hooks.pre_turn]]
command = "/usr/local/bin/check-turn"
args = ["--project", "kuru"]
timeout_ms = 5000
max_output_bytes = 65536

[[hooks.pre_tool]]
command = "/usr/local/bin/check-tool"

[[hooks.post_tool]]
command = "/usr/local/bin/record-tool"

[[hooks.speaker_selected]]
command = "/usr/local/bin/check-speaker"

[[hooks.post_turn]]
command = "/usr/local/bin/record-turn"
```

Each event accepts at most 16 commands. A command accepts at most 64 arguments;
its timeout is 1–120,000 milliseconds and its stdout limit is 1–262,144 bytes.
The defaults are 5,000 milliseconds and 65,536 bytes. A request is at most
256 KiB, stderr capture is at most 64 KiB, and one annotation is at most 16 KiB.
Each conversation or standalone dream operation also permits 1–1,024 hook
invocations (default 256), 1–600,000 milliseconds of shared active hook time
(default 120,000), and 1–1,048,576 aggregate annotation bytes (default 262,144).
Overlapping commands consume the shared wall-time budget once; time spent in
provider inference or ordinary tools does not consume it. Cleanup after timeout
or cancellation still holds the owned hook process and consumes active time.
Exhausted pre or speaker budgets prevent dispatch; exhausted post or annotation
budgets record a separate failure and leave settled work unchanged. A
higher-priority configuration layer replaces an
event's complete command array in normal TOML layering order.

Commands run in declaration order. `pre_turn` may allow, deny, or rewrite the
pending input. `pre_tool` may allow, deny, or rewrite only the proposed
arguments; a rewrite that names a different tool fails that hook, and no later
hook, permission evaluation, or dispatch follows. Kuru refuses a call that the
runtime would dispatch itself unless it names a tool offered for that request and
phase, whether the model proposed it or a hook rewrote it. Deliberation offers
only its cognition tools, and a dream offers only `dream_suggest`. Other speaking
calls go through the tool host, which checks their exact final name and
arguments. Every rewrite then passes the same input, schema, budget, root,
instruction, and permission checks as an unmodified value; a hook cannot reuse an
earlier grant or widen authority. `speaker_selected` may
observe or stop the already-validated selection. It cannot name another speaker
or cause a second selection. `post_tool` and `post_turn` may observe or annotate
settled work. A post failure does not change a tool effect, result, receipt,
usage, answer, or durable conversation, and later post hooks still run.
A rewritten `pre_turn` input replaces the original in every provider
projection of that turn: its own requests and a retry of it, every later turn's
public-transcript context for every part (including parts that did not take
part in the rewritten turn), resumed sessions, forks that inherit the turn, and
context compaction. The model therefore sees only the rewritten text as the
user's request, and its view of the conversation matches what it received. The
user-facing public transcript, session history, export and terminal view keep
the user's original input.

Kuru keeps two private provenance records for a rewrite, and neither is ever
sent to a provider:

- Each participating part's private history retains the rewritten input,
  preceded by a durable `kuru-hook` record (`event: pre_turn`,
  `outcome: rewritten`, with the session, operation, invocation, turn and
  rewriting hook indexes). Hook-authored text is therefore never stored as
  indistinguishable user speech. The retained rewritten text remains ordinary
  context for that part's later turns.
- Before any provider request, Kuru stores one turn-scoped record with the
  rewritten input and the same identities, but not the original. Every public
  transcript projection of that turn uses it in place of the original.

In a dream, a call must still be `dream_suggest` and pass the authored
proposal cap and dream validation. Dream annotations stay in the candidate
memory view and disappear if that candidate is abandoned.

Annotations are separate private hook records. Post-tool annotations are
eligible for the receiving actor's next request alongside the unchanged result;
post-turn annotations are eligible only for later context. Ordinary visibility
and context fitting may omit older annotations, and no annotation starts an
implicit provider turn. Completed exact retries return their stored outcome and
do not rerun hooks, providers, or tools. A refused `pre_turn` has no provider or
tool dispatch, so an explicit retry of that unfinished turn may run its pre
hooks again. Configured commands can have external effects; Kuru does not
promise exactly-once effects for a retried hook command.

Configured hook commands are reviewed process authority under [workspace
trust](#workspace-trust). Kuru retains the reviewed project directory, supplies a
finite compatibility environment without provider credentials, closes stdin
after one request, bounds and drains output, and owns the child process tree
through success, failure, timeout, cancellation, or caller loss. A turn, a dream,
and tool-host shutdown wait for owned hook trees to be reaped, including trees
whose callers were cancelled, before they return and before the project writer
lease can be released. Cleanup that cannot be confirmed is reported. After a
hook's own process exits, Kuru drains its output only until the hook deadline. A
descendant that left the owned process group while holding the output open
fails the hook instead of delaying the turn. This is not an
OS sandbox: an approved command runs with the user's filesystem, process, and
network authority. Hook protocol handling never invokes another lifecycle hook.
A Kuru run started by an owned hook command keeps ordinary trust, admission,
provider, and tool checks but suppresses lifecycle-hook dispatch for that nested
process, preventing the hook chain from starting itself again. Owned hook
launches mark their children with `KURU_INTERNAL_LIFECYCLE_HOOK_ORIGIN=1`. Any
Kuru process that inherits that value reports each configured hook as a
`suppressed` hook outcome at its lifecycle boundary and runs none of them. Do
not export the variable in an ordinary shell.

On Windows the finite hook environment keeps an inherited `PSModulePath`, so a
deliberate module setting reaches a generic hook command. The exception is a
hook that explicitly runs the system's stock Windows PowerShell
(`WindowsPowerShell\v1.0\powershell.exe`): Kuru removes the inherited value so
that edition reconstructs its standard module paths. Hooks do not receive the
built-in shell tool's `$PSHOME` module bootstrap, which is specific to that
tool. A cold stock PowerShell 5.1 start, such as the first hook after sign-in or
on a fresh profile, can exceed the 5,000-millisecond default. Set `timeout_ms`
on each Windows hook that runs stock PowerShell to allow for its cold start (up
to the 120,000-millisecond maximum), and keep the operation's `max_total_ms`
large enough for the hooks it runs.
See [hook protocol](protocols.md#lifecycle-hook-protocol) for the exact request
and decision shapes.

## Tool permissions

Tools use `allow`, `ask` or `deny` decisions after workspace trust and the
existing file-root checks. The legacy `allow_write` and `allow_shell` booleans
are fallback rules: `true` allows, while `false` asks in an attached interactive
session. A direct CLI command or unattended operation that needs approval
returns a permission-required refusal without executing or waiting for input.
Use an explicit `deny` rule when an operation must never be approved.

```toml
[[permissions]]
action = "ask"
selector = { kind = "native", name = "file_write" }
path = "src/**"

[[permissions]]
action = "deny"
selector = { kind = "native", name = "shell" }

[[permissions]]
action = "ask"
selector = { kind = "mcp", alias = "local_service", tool = "write_document" }

[[permissions]]
action = "ask"
selector = { kind = "a2a", alias = "research_peer" }
```

Selectors use exact native names (`file_read`, `file_list`, `grep`, `glob`,
`file_write`, `file_delete`, `shell`, `web_fetch`), an MCP configuration alias and original server tool
name, or an outbound A2A configuration alias. MCP's provider-facing hashed tool
name is not a permission selector. File patterns are anchored to the validated
project-relative path; they cannot authorize an outside or protected target.
Shell commands and MCP arguments do not have pattern matching.
`file_edit` is an exact-context mutation and uses the same `file_write` native
permission selector for its target; no separate path grant is inferred.

Up to 128 rules are supported. File patterns contain at most 512 Unicode
characters: `*` and `?` match within one path segment, while a whole-segment
`**` spans zero or more segments. `.` names the project root for listing.
Absolute paths, drive prefixes, backslashes, traversal, control characters and
bracket/brace pattern syntax are rejected.

Any matching `deny` wins, followed by `ask`, then `allow`, independent of rule
order. Only when no explicit rule matches do the legacy booleans apply. Ordinary
reads/listing and configured MCP/A2A calls otherwise remain allowed after trust.
A higher-priority configuration layer replaces the entire `permissions` array.

The TUI offers once, session, always and deny. Once covers the exact invocation.
Session and always cover the displayed scope: one exact file target, or the whole
shell/MCP/A2A tool. Filename metacharacters stay literal in these grants. Session
grants end with the running session, including before resume. Always grants are
stored privately outside project memory and bind the workspace's native identity,
reviewed authority manifest and effective permission/tool routes. A changed
authority context invalidates them. `/permissions` inspects and revokes grants;
explicit deny still wins over a saved grant. Closing or cancelling an unanswered
prompt does not authorize execution.
Tool execution stops if the checked project directory is renamed or replaced;
reopen Kuru from the intended directory to establish its current workspace context.
If an exact scope cannot be displayed without shortening or redaction, only
once and deny are available; Kuru will not remember a scope the prompt did not
fully show.

## Workspace trust

Kuru reviews effective process, mutation, endpoint and prompt authority supplied
by automatically discovered ancestor `.kuru/config.toml`, `AGENTS.md`,
`CLAUDE.md` and their checked imports
before activating it. This includes an `AGENTS.md` at the project root and any
automatic source under your home directory; location alone does not make a file
an explicit caller input. Kuru has no separate global-instruction source.
The trust subject is the exact canonical `-C` directory and its current native
directory identity. Approval does not inherit to parent or child directories.
User defaults, managed policy, the untracked project-local file, an explicit
`--config` file and CLI flags are deliberate caller inputs; an effective value
supplied by one of those layers does not require workspace approval. A later
explicit value can disable repository authority that is no longer effective;
it cannot reclassify a remaining repository-origin claim.

```sh
kuru -C /path/to/project trust status
kuru -C /path/to/project trust approve
kuru -C /path/to/project trust approve --yes
kuru -C /path/to/project trust revoke
kuru -C /path/to/project --trust-workspace-once tools
```

`trust status` displays the normalized root, automatic project sources, safe
claim descriptions and whether the complete current manifest matches its stored
approval. The project-instructions claim lists its bounded source labels in
outermost-to-most-local order and never prints instruction contents. `trust
approve` reviews and stores that complete manifest; `--yes` is the explicit
noninteractive form. Any effective ancestor authority addition, removal or
value change, or any changed ancestor/root instruction source, invalidates the
stored base approval. Changed nested sources invalidate their exact nested
grant and require review when reached; they do not silently widen the base.
A change only to mode, model,
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

When an actor first reaches nested `AGENTS.md`, `CLAUDE.md`, or their bounded
imports through an authorized file path, the terminal shows the new complete
manifest and offers **once**, **persist**, or **deny**. Nested persistent grants
are bound to the exact active path-source set and complete manifest in the same
root approval record; they do not widen the root-only startup grant. `trust
revoke` removes both base and nested grants. A changed or newly reachable
source needs fresh review. A headless `run` without a matching stored approval
or `--trust-workspace-once` returns a trust-required tool result with a review
remedy. A newly instructed write or delete has no effect on its first call:
Kuru settles that call and asks the actor to replan with the updated prompt.
Other tool calls from the same pre-discovery provider response also settle
without effects. An approved read or search can return its reviewed result.
An unsafe approval record still permits an explicit once choice, while
persistent approval requires revoking or repairing that record first.

The activation sets are command-specific:

| Commands | Automatic ancestor authority checked before activation |
| --- | --- |
| `login`, `logout`, `config`, `trust ...`, `update` | None; login/logout use the fixed ChatGPT route, config omits saved preferences, and revoke does not parse current workspace configuration |
| `auth` | Active Responses route; under another provider its API-key availability is not checked |
| `sessions`, `memory ...`, `undo-dream` | Configured memory executable and cache paths |
| `models` | Configured memory paths used while loading saved selections, plus an active Responses route |
| `tool` | Configured memory paths used while loading saved selections, built-in write/shell defaults, permission rules, and stdio/HTTP MCP configuration |
| `tools` | Built-in write/shell defaults, permission rules, and stdio/HTTP MCP configuration; catalog inspection does not activate memory paths |
| `run`, `dream`, `serve`, interactive TUI | All applicable project-instruction, memory, provider, lifecycle-hook, write, shell, permission-rule, MCP and external-agent claims |

The other rows do not construct peer prompts, so they do not consume the
project-instructions claim. Reading a snapshot for `config` or trust inspection
does not inject its instruction bytes or activate a provider.

Review output and configuration errors escape and bound source labels and hide
MCP arguments and environment values, URL queries and credential values. Approval
records live in checked private files under `<data-dir>/trust/workspaces`, outside
Dolt and the tool root, and contain digests rather than configuration values.

Workspace trust authorizes configuration; it does not restrict an approved
process or approve a tool's pending ask. Automatically discovered permission
rules are part of the reviewed manifest. Shell and stdio MCP processes retain
your account's process authority.
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
`max_parallel` bounds both peer work and concurrently running independent native
read calls; permission or instruction decisions can still force a call onto the
serial foreground path.
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
allow_tools = ["read_*", "search"]
deny_tools = ["*_secret"]

[mcp.remote_service]
url = "https://example.com/mcp"
header_env = { Authorization = "REMOTE_MCP_AUTH" }

[mcp.oauth_service]
url = "https://mcp.example.com/tools"

[mcp.oauth_service.oauth]
enabled = true
client_id = "kuru-native-client"
scopes = ["mcp.read", "mcp.write"]

[mcp.disabled_service]
enabled = false
command = "/absolute/path/to/disabled-server"
```

Stdio servers may have an `env` table; avoid storing credentials in shared
configuration. HTTP entries cannot contain process arguments or an `env` table.
Their optional `header_env` table maps bounded HTTP header names to environment
variable names; literal header values and protocol-owned headers are rejected.
Resolved values are limited to 16 KiB each and 64 KiB across one server.
Resolved values stay out of configuration, status, diagnostics, and discovery
cache records.

Enabled OAuth aliases require HTTPS and one configured `client_id`, one
`client_metadata_url`, or advertised dynamic registration. Configured scopes are
a ceiling over challenge/metadata authority. OAuth rejects static
`Authorization` and `Proxy-Authorization` references; validated nonauthorization
headers are sent only to the protected resource. Alias credentials are
generation-bound native secret-store records with no plaintext fallback. Browser
and advertised device login, redacted status, and local deletion are exposed by
the `mcp` command family; remote revocation is a separate reported outcome.

Servers default to `enabled = true`. A disabled server is reported without
resolving its headers, starting its process, connecting, or loading cached tools.
`allow_tools` and `deny_tools` match original server tool names with `*` and `?`;
deny wins, and a nonempty allow list omits unmatched names. Filtering happens
before routes and provider-facing names are built. Enabling a server gives the
harness access to its filtered tools after trust;
permission rules can then ask or deny individual calls. Kuru's built-in
`allow_write` and `allow_shell` fallbacks govern only its own tools. Call approval
does not replace the separate trust check before MCP startup.

Successful discovery writes bounded, versioned metadata to Kuru's owner-private
data directory outside the workspace. When a later live discovery fails, a valid
entry may appear as **stale** for inspection, but it never creates a route,
establishes availability, or grants a call. Cache state is bound to the exact
workspace authority, alias, endpoint, filters, header references, and resolved
header context. Invalid cache bytes are ignored when a healthy live discovery can
replace them.

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

Writes and shell execution require an effective allow rule, the corresponding
legacy opt-in flag, or a valid approval. Explicit deny overrides these grants.
Approving shell permits subprocess activity with your account's
permissions, including network access; the working directory does not constrain
what a subprocess can access. Built-in file tools separately enforce canonical
root containment, including symlinks, and protect instructions, configuration
and state paths.
