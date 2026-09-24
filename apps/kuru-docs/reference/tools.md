# Tools & permissions

Kuru's runtime executes tool proposals after applying permissions and bounds. A provider supplies inference; it does not take over the tool host.

## Discover and invoke

```sh
kuru -C /path/to/project tools
kuru -C /path/to/project tool file_read --args '{"path":"README.md"}'
```

`tools` and the terminal UI's `/tools` command return one shared catalog with
built-ins, filtered MCP metadata, and a `disabled`, `live`, `stale`, or
`degraded` state for every configured alias. Use the returned identifier for a
tool on a live MCP alias; names are generated to stay distinct across servers.
Stale cached metadata is inspectable but cannot be routed or treated as a grant.

## Built-in tools

A [`[[permissions]]` rule](./configuration#tool-permissions) can `allow`, `ask`, or `deny` any of these by exact name, and can additionally pattern-match the two file-mutation tools by anchored, project-relative path. Absent a matching rule, reads and listing stay allowed; `file_write`/`file_delete` fall back to `allow_write`, while `shell` and `web_fetch` ask for approval. Add an explicit `deny` rule to refuse a tool outright.

| Tool          | Arguments                        | Permission fallback              |
| ------------- | -------------------------------- | -------------------------------- |
| `file_read`   | `path`                           | Always allowed (project read)    |
| `file_list`   | `path`                           | Always allowed (project listing) |
| `grep`        | `pattern`, optional `path`       | Per-candidate project read       |
| `glob`        | `pattern`, optional `path`       | Per-candidate project discovery  |
| `file_write`  | `path`, `content`                | `allow_write` → ask              |
| `file_delete` | `path`                           | `allow_write` → ask              |
| `shell`       | `command`, optional `timeout_ms` | `allow_shell` → ask              |
| `web_fetch`   | `url`                            | Ask                              |

For an explicit write:

```sh
kuru --allow-write tool file_write \
  --args '{"path":"notes.txt","content":"A working note."}'
```

## Web fetch authority

`web_fetch` retrieves a UTF-8 document from a public HTTP(S) address only after the normal permission decision. It rejects URLs with credentials and rechecks every connection and redirect target, refusing loopback, private, link-local, multicast, unspecified, IPv4-mapped IPv6, carrier-grade NAT, and other special-purpose destinations. It follows at most five redirects, uses a 20-second operation deadline, accepts at most 2 MiB of response bytes, and returns at most 64 KiB. Its result records total `body_bytes`, retained `content_bytes`, `truncated`, and `untrusted: true`. Proxy, provider, and MCP credentials or headers are never forwarded.

Fetched text is untrusted tool data. It can inform the current response but cannot add instructions, change workspace trust, widen permissions, or authorize later tool calls. Kuru currently refuses compressed content rather than accepting an unbounded decompressor.

## Concurrent independent reads

If a provider proposes several independent reads that are already authorized,
Kuru can run `file_read`, `file_list`, `grep`, `glob`, and `web_fetch` calls
concurrently up to `max_parallel`. Permission and nested-instruction admission
still happen for each call in provider order. Progress can finish in a different
order, while the provider receives the final tool results in its original call
order and with the original call IDs.

A fresh approval or newly discovered instruction boundary stays on the normal
foreground path before later calls continue. File mutations, shell, MCP, skill
activation, cognition, and unknown tools remain serial. Kuru rechecks the
workspace, exact target, permission, and instruction scope before each read; a
change refuses that call instead of exposing data under stale authority.

## File boundaries

Paths are relative to the opened project. Absolute paths, parent traversal, and symlinks are rejected. Sensitive directories and configuration or credential files are protected; instruction files can be read but not mutated.

In an actor turn, an authorized file path can reveal nested project instructions. Kuru reviews their complete workspace manifest separately from file permission before returning a read/search result or applying a write. Grep and glob review only individually allowed candidate files. A newly instructed write/delete first returns a replan result without changing the file; the actor's next request receives the updated instructions. Direct `kuru tool` commands keep their explicit tool behavior without actor instruction activation.

The actor-only `skill_load` tool selects a name from the effective [skill catalog](./configuration#skills-and-custom-prompts) and optionally one direct Markdown reference. Kuru captures and, for project sources, reviews the selected prompt bytes before adding them to the next actor request. It does not execute skill scripts or grant the tools mentioned by a skill. Direct `kuru tool` calls cannot activate skills.

The built-in file tools enforce containment through capability-relative filesystem operations. UTF-8 file reads retain a marked 2 MiB head-and-tail excerpt after credential projection. The private state directory must remain outside the tool root.

## Shell authority

Shell execution goes through the permission engine. An explicit `deny` rule refuses it outright and can never be prompted around; an explicit `allow` rule runs it immediately. Absent a matching rule, `--allow-shell` or `allow_shell = true` allows it, and `allow_shell = false` (the default) asks for approval rather than blocking — a direct CLI command or other headless caller that hits that ask with no prompt to answer gets a structured permission-required refusal instead of executing or hanging. Once approved, it runs with your process permissions, including possible network access and access beyond the project. It is not a sandbox.

The built-in shell receives a finite compatibility subset of inherited variables
for command discovery, home/profile, temporary paths, locale, time, and standard
XDG locations. Provider/authentication variables, proxy configuration, agent
sockets, arbitrary `KURU_*` variables, and shell-startup controls are omitted.
Windows derives its stock system-shell paths and omits inherited `PSModulePath`,
allowing stock PowerShell to reconstruct its standard module paths. This
reduces accidental environment disclosure; it does not restrict filesystem,
process, or network authority. Before running your command, the Windows shell
loads the shipped `Microsoft.PowerShell.Management` and
`Microsoft.PowerShell.Utility` modules from `$PSHOME`. Other modules can still
load automatically. Configured stdio MCP servers retain their own inherited
environment and configured overrides.

Commands use `sh` on macOS/Linux and stock Windows PowerShell on Windows. The default timeout is 30 seconds. `timeout_ms` accepts 1–120000 milliseconds. Standard output and standard error each retain an independent marked 2 MiB head-and-tail excerpt after credential projection. On Unix, a registered owner retains the standard shell root, both pipes, and its fresh process group through cleanup; it signals that original group before reaping the root and confirms absence before normal completion. If bounded confirmation is unavailable, Kuru reports it while retaining the owner for later observation. A selected execution/startup deadline may use one additional five-second cleanup-confirmation allowance; delayed startup returns at the deadline plus that allowance, and host shutdown shares one five-second window across registered shells. This does not control processes that leave the original group. Windows retains its existing Job cleanup. MCP protocol framing stays hard-bounded at 2 MiB.

Before Kuru returns a built-in file or shell result, or an MCP success or
application error, it replaces recognized credential spans with
`[REDACTED:recognized-secret]`. The finite rules cover Basic/Bearer
Authorization and Proxy-Authorization values; OpenAI `sk-svcacct-`, `sk-proj-`,
then `sk-` prefixes, longest first, with at least 16 token bytes after that
prefix; GitHub `ghp_`, `github_pat_`, `gho_`, `ghu_`, `ghs_`, and `ghr_` forms
with at least 8 following token bytes; AWS `AKIA`/`ASIA` followed by exactly
16 uppercase-alphanumeric bytes; supported PEM private-key blocks; and exact
sensitive field names including `api_key`, `password`, and `refresh_token`.
Outward tool-failure details, including error formatting and source chains,
receive the same projection.
OpenAI and GitHub minimum lengths are local false-positive controls, not proof
that a value is valid. In JSON, an exact sensitive or Authorization key replaces
its whole value; otherwise Kuru replaces only recognized text spans.
AWS matching uses its uppercase-alphanumeric token alphabet for the trailing
boundary, so a following lowercase byte is outside that token shape.

The projection changes only the returned value. Tool arguments, writes, source
files, prior history, configured MCP environments, credential stores, and each
producer's own size limit stay unchanged. It does not discover arbitrary,
encoded, split, transformed, or future secret formats, so an unrecognized value
may remain visible; a matching ordinary string may be projected. Tool output is
therefore not a byte-exact backup. Stdio MCP stderr is scanned before a bounded,
terminal-escaped tail is retained for direct human diagnostics. The finite
scanner does not remove every ordinary URL, path, or configuration-like value.
Captured stderr is not placed in runtime events, model input, or memory.

## Third-party tools

Configured MCP servers bring their own authority. Kuru's built-in write and shell switches do not restrict what those services can do. Select services and credentials accordingly; [MCP configuration](./mcp) covers transport and lifecycle behavior.

MCP discovery publishes complete catalogs per configured alias. One unavailable
server does not hide built-in or healthy-server tools; its prior routes dispatch
nothing until a later explicit discovery succeeds. Transport failures and
ambiguous cancellation do not automatically retry mutating calls. MCP
application errors remain useful results and do not disable a healthy server.
Original-name allow/deny filters run before publication, with deny precedence.
Private discovery cache metadata is bound to the reviewed workspace and exact
server/authentication context; it contains no resolved static-header values.
