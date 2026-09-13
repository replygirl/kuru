## Context

Unix built-in shell launch currently inherits Kuru's complete environment.
Windows constructs its explicit native child environment from the complete
parent map and removes only `PSModulePath`. An authorized model-authored command
therefore receives provider keys, authentication and proxy variables, agent
sockets and unrelated process controls before the command asks for any of them.

The shell is still an explicit process-authority tool. It can access same-user
files and networks, so environment minimization narrows accidental value
disclosure without providing an OS sandbox. Configured stdio MCP has a separate
contract: parent inheritance plus deliberate validated overrides.

## Goals / Non-Goals

**Goals:**

- Pass a finite, documented compatibility environment to only the built-in
  shell on Unix and Windows.
- Preserve ordinary coding command discovery, profiles/home, temporary paths,
  locale, standard XDG paths and the stock Windows PowerShell baseline.
- Prove real child behavior with fake values without reading credentials or
  mutating the test runner's global environment.

**Non-Goals:**

- Filesystem, process or network containment.
- A configurable permission/environment language.
- Any MCP, provider, auth, updater, memory or general process-launch policy.
- A claim that the value of an allowlisted compatibility name cannot be secret.

## Decisions

### Project a finite connector-owned list

`kuru-connectors::tools` owns a private pure helper that projects an input
iterator of `(OsString, OsString)` pairs. Production captures
`std::env::vars_os()` once immediately before building the shell command. Unix
calls `env_clear` and applies the projection; Windows assigns the projected
vector to `NativeSpawnSpec.environment`. Root identity revalidation remains
immediately before spawn.

The exact inherited names are normative in the provider-tools delta. Unix name
matching is case-sensitive. Windows uses the existing ordinal case-insensitive
comparison, emits canonical spellings and rejects duplicate case-equivalent
inherited compatibility entries instead of choosing their values by iteration
order. Excluded entries and native-derived OS paths do not select an inherited
value, so their duplicates are ignored by the projection. Missing inherited
names remain missing except for the fixed Windows baseline below.

A denylist was rejected because every newly introduced secret variable would
inherit by default. A shared platform helper was rejected because this is policy
for a model-authored connector tool, not a generic process primitive.

### Preserve compatibility without advertising a terminal

Inherited `PATH` is deliberate executable-search authority for an already
authorized coding shell. Home/user, temp, locale, time, `NO_COLOR` and the five
XDG base-directory paths preserve common tools without adding language-specific
homes or flags. `TERM` and `COLORTERM` remain absent because stdin is null and
stdout/stderr are pipes; Kuru does not advertise terminal capability to a
captured subprocess.

On Windows, Kuru retains profile/appdata/program/architecture names and the
parent's sole `PATHEXT`. If `PATHEXT` is missing, it uses the existing
`.COM;.EXE;.BAT;.CMD` fallback. Kuru derives `SystemRoot` and `WINDIR` from the
parent of `system_directory()` and `ComSpec` from its checked `cmd.exe`, so an
inherited replacement cannot redirect the chosen stock shell or batch helper.
The absolute Windows PowerShell 5.1 program, `-NoProfile -NonInteractive` and
absence of `PSModulePath` remain unchanged.

`PATHEXT` was not replaced with a new fixed value when present: it participates
in the same explicit command-discovery behavior as `PATH`, and restricting the
user's registered extensions is unrelated to credential-variable disclosure.

### Preserve separate MCP semantics

The shell projection is called only from built-in Unix and Windows shell paths.
Configured stdio MCP continues to inherit its parent environment and apply
validated `McpConfig.env` overrides. A real child non-regression guards against
accidentally moving the projection into shared RPC/process code.

### Keep test instrumentation outside the shell environment

Pure projection tests use caller-supplied fake maps. Real shell tests spawn an
isolated outer test process with fake parent values, avoiding global environment
mutation. The shell reports presence/equality booleans rather than dumping the
environment. Shells may synthesize process-local values such as `PWD`; those are
separate from the exact Kuru launch projection.

Production never forwards `LLVM_PROFILE_FILE` to arbitrary shell commands. A
bare-command fixture uses a stock native utility, or its outer instrumented test
process explicitly retains its own coverage destination without adding that name
to the production list.

## Risks / Trade-offs

- Some tools depend on undocumented language, editor, certificate or corporate
  proxy variables. They will no longer receive those implicitly. A concrete
  compatibility need can add one reviewed finite name later; Phase 0 does not
  add a wildcard or configuration escape hatch.
- `PATH`, XDG/profile paths and other retained values still influence child
  behavior and can point at sensitive locations. The user explicitly granted
  process authority; documentation states that minimization is not containment.
- Windows environment spelling is case-insensitive. Canonical output plus
  duplicate rejection prevents silent order-dependent selection.

## Integration contract

The Unix fixture must execute an actual `sh -c` child through `ToolHost`. The
Windows fixture must execute the actual absolute stock Windows PowerShell 5.1
through `NativeSpawnSpec`, including native case handling, inherited and absent
`PATHEXT`, derived OS paths, bare stock command execution and module
reconstruction. Native Windows execution is required before merge;
cross-compilation is not equivalent.

The MCP control uses a real local stdio peer and observes only fake inherited and
configured values. All fixtures preserve existing output bounds, deadlines,
cleanup assertions and retained-root revalidation, and print no real parent
environment value. The Unix check does not claim synchronous descendant-group
disappearance beyond the current implementation's observed contract.
