## Why

The authorized built-in shell currently inherits Kuru's complete process
environment, so model-authored commands can receive provider keys, authentication
variables, proxy credentials and unrelated process configuration without those
values being needed for ordinary command execution. This makes an explicit shell
grant disclose more ambient variable data than its documented process-authority
contract requires on both Unix and Windows.

## What Changes

- Give only the built-in shell a finite platform-specific compatibility
  environment for command discovery, home/profile, temporary paths, locale,
  standard XDG paths and stock Windows PowerShell operation.
- Preserve inherited `PATH` and Windows `PATHEXT` command-search authority while
  deriving Windows system-shell paths from the selected native OS directory;
  omit credentials, proxies, SSH-agent handles, arbitrary `KURU_*` entries and
  startup-injection variables.
- Keep configured stdio MCP environment inheritance and overrides unchanged.
- Add isolated Unix and native Windows environment/command fixtures and update
  the tool description and user documentation without claiming filesystem,
  process or network containment.

## Capabilities

### New Capabilities

### Modified Capabilities

- `provider-tools`: Narrow the ambient environment passed to an explicitly
  authorized built-in shell while preserving its documented process authority,
  compatibility inputs, bounds and platform cleanup behavior.

## Impact

The change is private to `packages/kuru-connectors` shell launch policy and its
Unix/native-Windows fixtures, plus the existing tools documentation in `docs`
and `apps/kuru-docs`. It adds no public API, configuration, dependency, MCP
behavior, permission language or filesystem sandbox.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [x] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [x] agent-behavior — prompts, tools, model routing, or agent output shape
