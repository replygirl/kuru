## MODIFIED Requirements

### Requirement: Explicit bounded tools

Filesystem tools MUST enforce canonical workspace containment and protect
instruction/configuration paths. Mutations and shell execution MUST require
explicit opt-ins and matching workspace approval when automatic ancestor
configuration contributes their effective grant. Shell execution MUST have time
and output bounds and SHALL be described as process authority rather than a
filesystem sandbox; workspace approval does not widen tool roots or make
private same-user state inaccessible to a shell.

The built-in shell MUST receive only this finite inherited compatibility
environment when each entry exists. Unix entries are `PATH`, `HOME`, `USER`,
`LOGNAME`, `TMPDIR`, `TMP`, `TEMP`, `LANG`, `LC_ALL`, `LC_COLLATE`, `LC_CTYPE`,
`LC_MESSAGES`, `LC_MONETARY`, `LC_NUMERIC`, `LC_TIME`, `TZ`, `NO_COLOR`,
`XDG_CONFIG_HOME`, `XDG_CACHE_HOME`, `XDG_DATA_HOME`, `XDG_STATE_HOME` and
`XDG_RUNTIME_DIR`. Windows entries are that same set plus `USERNAME`,
`USERPROFILE`, `HOMEDRIVE`, `HOMEPATH`, `APPDATA`, `LOCALAPPDATA`, `ProgramData`,
`ProgramFiles`, `ProgramFiles(x86)`, `ProgramW6432`, `PROCESSOR_ARCHITECTURE`,
`PROCESSOR_ARCHITEW6432` and `PATHEXT`, using ordinal case-insensitive names.
Windows MUST derive `SystemRoot`, `WINDIR` and `ComSpec` from the native system
directory instead of inheriting their values, and MUST use the conventional
`.COM;.EXE;.BAT;.CMD` `PATHEXT` only when the parent has none. Case-equivalent
duplicates of inherited compatibility names MUST fail deterministically rather
than select an ambiguous value. Duplicates of excluded names and native-derived
OS path names MUST NOT affect the projection.

Every other inherited entry, including provider/authentication values, proxy
configuration, SSH-agent handles, arbitrary `KURU_*` entries and shell startup
inputs, MUST be absent. `PSModulePath` MUST remain absent so the selected stock
Windows PowerShell reconstructs its standard modules. This variable reduction
MUST NOT be described as containing the shell's filesystem, process or network
authority, and an allowlisted name MUST NOT be treated as proof that its value
is nonsecret. Configured stdio MCP inheritance and `McpConfig.env` overrides
MUST remain unchanged.

The built-in Unix shell MUST register an independent owner before launch and
retain its root child, stdout/stderr readers, and exact workspace `Directory`
through cleanup. Its public timeout MUST begin at invocation acceptance and its
caller wait MUST end within that operation deadline plus one five-second
cleanup-confirmation allowance. Natural completion MUST require both pipe EOFs
and non-reaping root-exit observation, terminate remaining members of the
original group before reaping the root, preserve that root's original status,
and observe group absence before returning success. Timeout, overflow, read
failure, caller loss, parent-runtime loss, and shutdown MUST enter the same
owned cleanup path without replacing the primary failure.

`ToolHost` shutdown MUST close shell registration, request cancellation, and
await all registered owners within one bounded observation window while still
running MCP cleanup. A bounded unconfirmed result MUST leave the independent
worker holding its process, pipe, and workspace capability until later reap and
absence confirmation; it MUST NOT claim synchronous cleanup. This ownership is
limited to the built-in shell and MUST NOT claim control of escaped processes,
the memory writer lease, or configured MCP lifecycles.

#### Scenario: Symlink escape
- **WHEN** a filesystem call follows a workspace symlink outside the root
- **THEN** the operation fails without modifying the outside file.

#### Scenario: Pending shell or write grant
- **WHEN** an automatic ancestor enables shell or writes without matching approval
- **THEN** the tool host neither exposes nor executes that authority.

#### Scenario: Authorized shell receives finite compatibility environment

- **WHEN** an authorized built-in shell starts from a parent containing the documented compatibility entries plus fake provider, proxy, SSH-agent, Kuru, loader and startup-injection values
- **THEN** the real child observes the exact applicable compatibility values and none of the other parent entries, while its ordinary cwd, command discovery, output, timeout and cleanup behavior remains available.

#### Scenario: Stock Windows shell uses native baseline

- **WHEN** the built-in shell starts native Windows PowerShell with case-varied environment names, an inherited `PATHEXT` or no `PATHEXT`, and hostile replacements for OS shell paths and `PSModulePath`
- **THEN** it uses the native-derived system paths, preserves the sole inherited `PATHEXT` or the conventional fallback, rejects case-equivalent ambiguity, reconstructs stock modules and receives no `PSModulePath`.

#### Scenario: Configured stdio MCP retains its environment contract

- **WHEN** a configured stdio MCP starts after built-in shell minimization with one fake inherited non-allowlisted sentinel and one explicit `McpConfig.env` override
- **THEN** the MCP child receives both under its existing contract rather than the built-in shell projection.

#### Scenario: Unix root exits with a silent descendant
- **WHEN** both shell pipes close and the root exits while a same-group descendant remains alive
- **THEN** Kuru terminates the remaining group before root reap and returns the exact root status only after group absence is observed.

#### Scenario: Unix shell caller disappears
- **WHEN** a shell call or its parent Tokio runtime disappears after launch
- **THEN** the independent registered owner retains the child, pipes, and workspace capability through bounded cleanup, and `ToolHost` shutdown observes confirmation or reports that ownership remains unconfirmed.

#### Scenario: Unix worker is delayed before launch
- **WHEN** a registered worker is delayed beyond the accepted operation deadline and cleanup allowance while no child has spawned
- **THEN** the caller returns a fixed bounded cancellation or unconfirmed-cleanup result, and the later worker observes cancellation and starts no child.

#### Scenario: Unix ownership observation remains interrupted
- **WHEN** repeated bounded `EINTR` leaves an owned root anchored beyond the caller's cleanup allowance
- **THEN** the caller receives a fixed unconfirmed result while the registered worker retains ownership, later makes its one destructive transition after a valid observation, and never signals again after that transition starts.
