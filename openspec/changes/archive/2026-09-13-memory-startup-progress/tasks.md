## 1. Observed verified memory open

- [x] 1.1 Add fixed `MemoryOpenStage` and bounded observed-open receiver/future while preserving silent `MemoryStore::open`, and verify observer loss cannot retain memory ownership in focused store tests.
- [x] 1.2 Thread a private reporter through provisioning, startup lock, migration/activation, and server-open paths without changing digest, version-probe, lifecycle, or validation order; verify cold/warm/corrupt/failed real fixtures report exact stages and no false ready.

## 2. CLI progress and legacy import guidance

- [x] 2.1 Add one CLI observed-open helper that drains fixed stderr progress before normal command/TUI flow while preserving stdout/JSON and early no-store behavior; verify isolated cold/warm JSON and provider-free notes fixtures.
- [x] 2.2 Add actionable Unix 0700 legacy-directory refusal and native Windows owner-privacy guidance without automatic repair; verify a real legacy SQLite plus WAL CLI import preserves original layout and unsafe directories remain unchanged.
- [x] 2.3 Update owning memory/configuration documentation and verify docs checks describe stages as current work rather than integrity or completion estimates.

## 3. Focused acceptance evidence

- [x] 3.1 Run focused memory and TUI validation with prepared supervisor inputs, record actual cold/warm timing and native Windows/coverage limitations, and verify PTY restoration separately from plain stderr output.
