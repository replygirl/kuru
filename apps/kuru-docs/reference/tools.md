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

The default timeout is 30 seconds. `timeout_ms` accepts 1–120000 milliseconds. Standard output and standard error are each bounded to 2 MiB; Unix process groups are terminated on timeout or cancellation.

## Third-party tools

Configured MCP servers bring their own authority. Kuru's built-in write and shell switches do not restrict what those services can do. Select services and credentials accordingly; [MCP configuration](./mcp) covers transport and lifecycle behavior.

Tool discovery and calls report unavailable servers and malformed responses as errors. Mutating MCP calls are not automatically retried after transport failure.
