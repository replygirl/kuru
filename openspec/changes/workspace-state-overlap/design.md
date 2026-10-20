# Design

## Context

`paths` takes the workspace from `-C` (default the invocation directory) and the data directory from `--data-dir`, `KURU_DATA_DIR` or the platform default under the home directory, so a home-rooted workspace contains the data directory by construction. The approval, tool-grant, authentication, managed-configuration and memory stores each refuse that containment independently. The first one reached is trust preflight: `ApprovalStore::inspect` → `open_store` raises a containment `ensure!` with no OS error code, which `failed_read_state` classifies as `Invalid`; when the data directory does not exist yet `open_store` returns `None` first and the state is `Absent`. Either way the interactive prompt is offered and both choices fail later with different messages. Memory used two different checks: a canonical `starts_with` before the runtime open and `ensure_outside_workspace` (`is_within`) elsewhere. Doctor had no containment observation.

## Goals / Non-Goals

**Goals:** one early, read-only, identity-based detection and one clear message; a distinct trust and doctor classification; unchanged store invariants.

**Non-Goals:** decoupling the tool root from the workspace, relocating the default data directory, a reverse (workspace inside data) check, or weakening any store's own containment check.

## Decisions

- `trust::state_inside_workspace(data, root)` walks from the data path to its nearest existing ancestor with `Directory::open` (no-follow, native identity, creates nothing) and returns `is_within(root)`, mirroring the authentication store's walk. The early refusal classifies only a positive result, so an uninspectable path (for example one through a symbolic link) is not misreported as contained there; the per-store and memory open checks below fail closed on it instead.
- `execute_inner` runs it immediately after retaining the workspace, before inventory, login/logout, canary, revoke, file checkpoints, configuration discovery and preflight. `--trust-workspace-once` cannot bypass it. Headless `run` uses `RunExit(2)` (invalid invocation), not the trust-refusal status 3, because no approval can make the invocation valid.
- Carve-out: `config` and `trust status` still run. Both are pure inspections that never open the data directory; the workspace-trust spec requires `config` to print a snapshot against an absent data directory, and `trust status` is where the distinct status must be shown. Rejected alternative: refusing them too, which would contradict that spec scenario and leave no command that explains the state in context.
- `ApprovalState::InsideWorkspace` is returned by `inspect` and `inspect_nested` before the record read, so it is reported whether or not the data directory exists. `trust status` keeps exit 0 for every state, as it does for "not approved". The `open_store`/`create_store` `ensure!`s remain unchanged.
- Doctor computes the observation once, independently of the manifest (a workspace with no claims would otherwise read "approval not required"), and reports Workspace trust as Problem with `state_inside_workspace`/`separate_state_directory`; ChatGPT subscription and project memory report Unverified with the same condition and action instead of inspecting state that the launch would refuse. Fixed text only; no paths in doctor output.
- Memory keeps self-defending, identity-based checks (review round): `ensure_state_outside_workspace` reruns the same helper just before `MemoryStore::exists` and at each former site (legacy import, backup verify/restore after retaining the directory, writer and purge after `ensure_private`, the read-only existing-store open, and the runtime open on the canonicalized path). It refuses with the same message on containment and fails closed, with a message naming the data path and the remedy, when the path cannot be opened without following links. This replaces the two differing old checks (canonical `starts_with` and `ensure_outside_workspace`) with one implementation, and a rename between the early check and an open is re-validated at the open.
- Observed: `MemoryStore` refuses a linked data path on its own. On 2eeba124 (early check only), `sessions` with `--data-dir` linked to a store inside the workspace failed in `MemoryStore::exists`/`read_activation` with "project memory has no activation record; preserve the store and repair it before opening" (ENOTDIR, no-follow open). That failure is closed but misleading, so the re-check precedes `exists`. Writer paths were already refused by `Directory::ensure_private`'s no-follow descent, and the approval, grant, auth, checkpoint and MCP-cache stores each fail closed on such a path.

## Operational surface

Local CLI/TUI only: no bind address, container, secret, network, connection limit or binary-version change. The check runs on every supported OS through `kuru-platform` directory identities (Unix inode/device, Windows file IDs); Windows defaults (`%LOCALAPPDATA%\kuru` under the profile) produce the same refusal for a profile-rooted workspace. Native CI exercises the Unix PTY path; Windows runs the same child-process tests.

## Risks / Trade-offs

- [The early check precedes data-directory creation] → creation uses no-follow descent from the volume root, and every store, memory included, re-checks the retained or created directory's identity just before opening.
- [A read-only memory command with an unopenable data path, such as one containing `..`, now fails at the re-check instead of possibly listing nothing] → such paths already fail for every writer and store; failing closed matches the removed `ensure_outside_workspace`.
- [Commands such as `trust revoke` and `memory inventory` now refuse in this configuration] → they could not succeed against a contained store before either; the message gives the remedies.
