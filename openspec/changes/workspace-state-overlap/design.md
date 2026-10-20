# Design

## Context

`paths` takes the workspace from `-C` (default the invocation directory) and the data directory from `--data-dir`, `KURU_DATA_DIR` or the platform default under the home directory, so a home-rooted workspace contains the data directory by construction. The approval, tool-grant, authentication, managed-configuration and memory stores each refuse that containment independently. The first one reached is trust preflight: `ApprovalStore::inspect` → `open_store` raises a containment `ensure!` with no OS error code, which `failed_read_state` classifies as `Invalid`; when the data directory does not exist yet `open_store` returns `None` first and the state is `Absent`. Either way the interactive prompt is offered and both choices fail later with different messages. Memory used two different checks: a canonical `starts_with` before the runtime open and `ensure_outside_workspace` (`is_within`) elsewhere. Doctor had no containment observation.

## Goals / Non-Goals

**Goals:** one early, read-only, identity-based detection and one clear message; a distinct trust and doctor classification; unchanged store invariants.

**Non-Goals:** decoupling the tool root from the workspace, relocating the default data directory, a reverse (workspace inside data) check, or weakening any store's own containment check.

## Decisions

- `trust::state_inside_workspace(data, root)` walks from the data path to its nearest existing ancestor with `Directory::open` (no-follow, native identity, creates nothing) and returns `is_within(root)`, mirroring the authentication store's walk. Only a positive result classifies; an inspection error falls through to the existing checks so symlinked, unreadable or invalid data paths keep their current errors and states.
- `execute_inner` runs it immediately after retaining the workspace, before inventory, login/logout, canary, revoke, file checkpoints, configuration discovery and preflight. `--trust-workspace-once` cannot bypass it. Headless `run` uses `RunExit(2)` (invalid invocation), not the trust-refusal status 3, because no approval can make the invocation valid.
- Carve-out: `config` and `trust status` still run. Both are pure inspections that never open the data directory; the workspace-trust spec requires `config` to print a snapshot against an absent data directory, and `trust status` is where the distinct status must be shown. Rejected alternative: refusing them too, which would contradict that spec scenario and leave no command that explains the state in context.
- `ApprovalState::InsideWorkspace` is returned by `inspect` and `inspect_nested` before the record read, so it is reported whether or not the data directory exists. `trust status` keeps exit 0 for every state, as it does for "not approved". The `open_store`/`create_store` `ensure!`s remain unchanged.
- Doctor computes the observation once, independently of the manifest (a workspace with no claims would otherwise read "approval not required"), and reports Workspace trust as Problem with `state_inside_workspace`/`separate_state_directory`; ChatGPT subscription and project memory report Unverified with the same condition and action instead of inspecting state that the launch would refuse. Fixed text only; no paths in doctor output.
- The memory `starts_with` check and `ensure_outside_workspace` are removed; every memory path is downstream of the early check.

## Operational surface

Local CLI/TUI only: no bind address, container, secret, network, connection limit or binary-version change. The check runs on every supported OS through `kuru-platform` directory identities (Unix inode/device, Windows file IDs); Windows defaults (`%LOCALAPPDATA%\kuru` under the profile) produce the same refusal for a profile-rooted workspace. Native CI exercises the Unix PTY path; Windows runs the same child-process tests.

## Risks / Trade-offs

- [The early check precedes data-directory creation] → creation uses no-follow descent from the volume root, and approval, grant and authentication stores still check the created directory's identity. Memory relies on the early check alone; a same-user process moving directories between the check and creation is outside the threat model.
- [Commands such as `trust revoke` and `memory inventory` now refuse in this configuration] → they could not succeed against a contained store before either; the message gives the remedies.
