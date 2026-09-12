# Tools & permissions

Kuru's runtime executes tool proposals after applying permissions and bounds. A provider supplies inference; it does not take over the tool host.

## Discover and invoke

```sh
kuru -C /path/to/project tools
kuru -C /path/to/project tool file_read --args '{"path":"README.md"}'
```

`tools` lists the built-ins and tools from configured MCP servers. Use the returned identifier for an MCP tool; names are generated to stay distinct across servers.

## Built-in tools

| Tool          | Arguments                        | Permission      |
| ------------- | -------------------------------- | --------------- |
| `file_read`   | `path`                           | Project read    |
| `file_list`   | `path`                           | Project listing |
| `file_write`  | `path`, `content`                | `allow_write`   |
| `file_delete` | `path`                           | `allow_write`   |
| `shell`       | `command`, optional `timeout_ms` | `allow_shell`   |

For an explicit write:

```sh
kuru --allow-write tool file_write \
  --args '{"path":"notes.txt","content":"A working note."}'
```

## File boundaries

Paths are relative to the opened project. Absolute paths, parent traversal, and symlinks are rejected. Sensitive directories and configuration or credential files are protected; instruction files can be read but not mutated.

The built-in file tools enforce containment through capability-relative filesystem operations. File and output payloads have 2 MiB bounds. The private state directory must remain outside the tool root.

## Shell authority

Shell execution requires `--allow-shell` or `allow_shell = true`. It runs with your process permissions, including possible network access and access beyond the project. It is not a sandbox.

The built-in shell receives a finite compatibility subset of inherited variables
for command discovery, home/profile, temporary paths, locale, time, and standard
XDG locations. Provider/authentication variables, proxy configuration, agent
sockets, arbitrary `KURU_*` variables, and shell-startup controls are omitted.
Windows derives its stock system-shell paths and omits inherited `PSModulePath`,
allowing stock PowerShell to reconstruct its standard module paths. This
reduces accidental environment disclosure; it does not restrict filesystem,
process, or network authority. Configured stdio MCP servers retain their own
inherited environment and configured overrides.

Commands use `sh` on macOS/Linux and stock Windows PowerShell on Windows. The default timeout is 30 seconds. `timeout_ms` accepts 1–120000 milliseconds. Standard output and standard error are each bounded to 2 MiB. Timeout or cancellation terminates the owned Unix process group or Windows Job, including descendants.

## Third-party tools

Configured MCP servers bring their own authority. Kuru's built-in write and shell switches do not restrict what those services can do. Select services and credentials accordingly; [MCP configuration](./mcp) covers transport and lifecycle behavior.

Tool discovery and calls report unavailable servers and malformed responses as errors. Mutating MCP calls are not automatically retried after transport failure.
