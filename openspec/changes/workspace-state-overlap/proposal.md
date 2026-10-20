# Proposal

## Why

Kuru's workspace is the `-C` directory, by default the invocation directory, and its default data directory lives under the home directory. Running `kuru` from the home directory therefore makes the workspace contain Kuru's own state, which every store correctly refuses, but the refusal arrives late and mislabelled: trust preflight folds the approval store's containment `ensure!` into `ApprovalState::Invalid`, prints "approval state is invalid or unsafe" for a record that does not exist, and still offers the interactive prompt. "Approve" then fails with "approval storage must be outside the workspace", and "Continue once" fails later at memory with a different message. `kuru doctor` reports "stored approval is invalid" and an unexplained unverified ChatGPT check (observed with v0.11.0 on 2026-10-19, `cd ~ && kuru`).

## What Changes

- Detect a workspace that contains the data directory once, read-only, right after the workspace directory is retained and before configuration discovery, using the native identity of the data directory or its nearest existing ancestor. Every command past that point refuses with one message naming both directories (bounded and escaped) and the remedies: run from a project directory, pass `-C PROJECT`, or set `--data-dir`/`KURU_DATA_DIR` outside the workspace. Headless `run` exits 2 (invalid invocation). The interactive trust prompt is never shown in this situation.
- `trust status` and `config` remain pure inspections and still run: `trust status` reports a distinct `ApprovalState::InsideWorkspace` ("approval storage lies inside this workspace" plus remedies). `Invalid` stays reserved for malformed or unsafe records.
- `kuru doctor` reports a distinct `state_inside_workspace` condition with a `separate_state_directory` action under Workspace trust (Problem, exit 3), and names the same cause for the ChatGPT subscription and project memory checks instead of an unexplained unverified result.
- Replace the two differing memory containment checks (canonical `starts_with` and per-call `is_within`) with the single early check. Approval, grant, authentication and managed-configuration stores keep their own checks unchanged as defense in depth.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

`apps/kuru-tui/src/cli.rs`, `trust.rs`, `doctor.rs`, their tests, and user docs (`docs/install.md`, `docs/configuration.md`, `apps/kuru-docs` installation, troubleshooting and configuration pages). No change to the data-directory default, the tool root, memory, runtime, providers, dependencies or store formats. The doctor JSON schema stays version 1 with one additive condition and action.

## Surfaces

- [x] interactive — CLI/TUI startup refusal, trust status wording and doctor report
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
