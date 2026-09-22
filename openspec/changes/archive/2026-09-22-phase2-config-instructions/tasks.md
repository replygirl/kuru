## 1. Managed and local sources

- [x] 1.1 Implement strict external managed document parsing, defaults merge and exact-value constraint checks in the immutable config snapshot; verify ordinary defaults, atomic locks and conflicting final values in core tests.
- [x] 1.2 Implement checked discovery of the canonical project-local file with bounded Git index classification; verify untracked, tracked, non-Git and ambiguous repository cases through isolated CLI child processes.

## 2. Invocation layers

- [x] 2.1 Add typed repeatable `-c` parsing, strict key/type checking, final-leaf provenance and dedicated-flag precedence; verify nested scalar, map, array, unknown-key and invalid-type cases.
- [x] 2.2 Wire all layers through CLI snapshot capture and post-memory saved preferences without authority changes after trust preflight; verify remaining repository claims, explicit disabling, managed conflicts and redacted `config` output in CLI integration tests.

## 3. Public contract and acceptance

- [x] 3.1 Update the published configuration schema and user documentation for managed, local and `-c` behavior; verify schema/parser parity and `mise run docs:check`.
- [x] 3.2 Run the planned integration and repository checks, record observed results in `verification.md`, and validate the change strictly before archive.
