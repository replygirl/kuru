# Verification

## 1. Early clear refusal [critical]

- [x] 1.1 @regression (agent) real-PTY interactive launch with the data directory inside the workspace (explicit and default-derived) -> before: trust prompt shown; after: exits before the alternate screen with the single "outside the tool workspace" message and no "Continue once". Observed: on b1339bdc the test timed out at "Choice [3]:" after "Status: not approved"; after the fix it passes for explicit `--data-dir` and `XDG_DATA_HOME` derivation, with and without claims, and creates no nested directory.
- [x] 1.2 @e2e (agent) child-process `sessions`, `trust approve --yes`, `logout`, `--trust-workspace-once sessions` and `run` against a containing workspace -> each refuses with the same message (run exit 2) and creates no data directory; `config` still prints TOML. Observed: `a_workspace_containing_the_data_directory_is_refused_before_review` passes (also `trust revoke`, `memory inventory`, `file list`; exit 1, run exit 2) for absent and existing nested data; the nested-data preferences test passes unchanged.

## 2. Distinct classification

- [x] 2.1 @unit (agent) `ApprovalStore::inspect`/`inspect_nested` with absent and present data inside the root -> `InsideWorkspace`; a malformed record outside the root -> `Invalid`. Observed: both trust.rs unit tests pass; a linked data path is not classified as contained.
- [x] 2.2 @e2e (agent) child-process `trust status` -> exit 0 and "Status: approval storage lies inside this workspace" with remedies; nothing created. Observed in the same child-process test.
- [x] 2.3 @e2e (agent) child-process `doctor` and `doctor --json` with and without manifest claims -> exit 3, Workspace trust `state_inside_workspace`/`separate_state_directory`, ChatGPT subscription and memory name the same cause, no private paths. Observed: `doctor_names_a_workspace_that_contains_the_data_directory` passes without claims, with a claim, with invalid configuration and with existing data.

## 3. Preservation

- [ ] 3.1 @integration (agent) existing trust, preferences, doctor and CLI suites plus host/Windows lint, typecheck, docs and cospec checks -> pass; CI coverage gate holds at 95%. Observed locally: app suite 447/0, all static checks exit 0. CI run 39446598749 on 86ce5873: 60 checks pass; coverage macOS 95.23%, Ubuntu 95.22%, Windows 95.01% (gate 95%). Open: `quality / RustSec advisories` fails on externally published rustls (RUSTSEC-2026-0346..0349) and zip (RUSTSEC-2026-0367..0369) advisories unrelated to this change, so the required green run is not yet observed.
