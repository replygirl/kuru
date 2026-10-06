# Proposal

## Why

Unix self-update currently writes into the running executable's directory without recognizing package-manager ownership. A user-writable mise or Homebrew installation can therefore be replaced outside its owning manager, violating the settled D2/P24 update policy.

## What Changes

- Classify the resolved executable path against component-exact mise and Homebrew installation roots.
- Refuse every Unix self-update mode before network requests, builds, staging or recovery-state creation when an owning package manager is detected; name its upgrade command.
- Retain checked non-manager installation objects and reject unsafe destination identities before effects without changing Windows update behavior.
- Document the refusal and verify actual copied executables, aliases, no-effect inventories and fake request/build counters.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `repository-delivery`: package-manager ownership and pre-effect self-update refusal.

## Impact

Delivery ownership module; narrow safe platform filesystem ownership/access preflight if existing retained APIs cannot express it; Unix CLI update entrypoint; delivery/CLI native fixtures and install/command documentation. No Windows receipt/bootstrap changes, new dependencies/pins, schema, provider authority, release workflow, Homebrew packaging, Unix recovery or update notices in this first change.

## Surfaces

- [x] interactive — actionable CLI refusal
- [ ] deploy — no install layout or release topology change
- [x] integration — existing package-manager installation ownership
- [ ] agent-behavior — no actor/provider behavior change
