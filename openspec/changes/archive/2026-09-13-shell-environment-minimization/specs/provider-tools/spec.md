## MODIFIED Requirements

### Requirement: Explicit bounded tools

Filesystem tools MUST enforce canonical workspace containment and protect
instruction/configuration paths. Mutations and shell execution MUST require
explicit opt-ins and matching workspace approval when automatic ancestor
configuration contributes their effective grant. Shell execution MUST have time
and output bounds and SHALL be described as process authority rather than a
filesystem sandbox; workspace approval does not widen tool roots or make private
same-user state inaccessible to a shell.

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
