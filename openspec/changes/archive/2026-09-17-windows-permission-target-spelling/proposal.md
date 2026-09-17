## Why

On Windows, a checked file tool can fail before permission authorization because the expanded parent path is compared with the retained root using different path prefixes. The resulting built-in error masks the required typed permission refusal and blocks otherwise permitted file operations.

## What Changes

Compare the expanded file target with a canonical spelling of the retained root while keeping the held-directory identity check and protected-component checks. Cover ordinary and verbatim Windows root spellings, a permitted write, and an unattended ask refusal with no file effect.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

`packages/kuru-connectors/src/tools.rs` and focused connector tests; the CLI permission fixture gains a safe diagnostic on assertion failure. No public API, dependency, migration, or policy change.

## Surfaces

- [x] interactive — file-tool permission errors and permitted file operations are user-visible
- [ ] deploy
- [ ] integration
- [x] agent-behavior — native file-tool dispatch and permission refusal
