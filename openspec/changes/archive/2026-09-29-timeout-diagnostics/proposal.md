# Proposal

## Why

Four CI failure families cannot be pinned to a command. The install timeout
(`embedded_runtime.rs` at 100 s and `windows_cli.rs` at an implicit 180 s), the
git child 10 s timeout (`advisory.rs` and `coverage.rs` test helpers) and the
terminal child exit timeout (`Terminal::wait_exit`) each fail with a message
that names a pid, or nothing, but not the command, its arguments, its working
directory or the processes still alive. A fourth family appeared on
macos-latest: `terminal_fixture_uses_its_requested_controlling_dimensions`
fails with `nested terminal reports its own size: process exited ExitStatus {
code: 0 }`, meaning the nested child exited early and cleanly before writing
its size report, and the error carries neither the child's PTY output nor what
the fixture printed.

On Windows this cannot be fixed in tests alone. Every Windows timeout in the
install and git families goes through one timeout arm in shipped source,
`packages/kuru-delivery/src/command.rs`, which terminates the process tree
before it returns. The Job handle is private to `NativeChild`, so no test helper
can observe the tree afterwards, and the platform exposes no list of the Job's
processes. Reimplementing the timed spawn per helper would still need the same
platform accessor.

## What Changes

Diagnostics only. Deadlines, retries, sleeps, assertions and behaviour are
unchanged; the change adds context to error text on failure paths.

- The shared timeout arm in `command.rs` adds, to its error text only and
  captured before it terminates the tree, the root state, the Job state and the
  process IDs in the Job with image names. The Unix arm gains the same for
  symmetry, with the descendant snapshot taken before the child is killed.
- `kuru-platform`'s audited Windows interop module gains a small read-only
  `NativeChild` accessor (`QueryInformationJobObject` with
  `JobObjectBasicProcessIdList`, optionally `OpenProcess` with
  `PROCESS_QUERY_LIMITED_INFORMATION` and `QueryFullProcessImageNameW`). Process
  IDs are diagnostic text; nothing acts on a numeric PID.
- Test helpers attach the command, arguments, working directory and elapsed
  time to each timeout: the Unix and Windows `execute` in `embedded_runtime.rs`,
  `windows_cli.rs`, the five git helpers plus the delivery fixture, and
  `Terminal::wait_exit` (with a bounded `ps` snapshot of the child and its
  descendants). The two negative terminal tests keep the substrings they assert.
- Family 4: the nested terminal fixture test captures the nested child's complete
  PTY output and exit status in its error and records what the nested fixture
  printed.
- Every snapshot runs only on the failure path, is bounded, drains helper
  output, and on failure appends `snapshot unavailable: <reason>` without
  replacing or hiding the original error.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. A search of `openspec/specs` found no durable requirement that states
timeout error text or process-diagnostic content (`native-platform` and
`native-windows` constrain descendant quiescence and Job ownership, which this
change does not alter), so there is no delta spec and the specs artifact is
omitted.

## Impact

- `packages/kuru-delivery/src/command.rs` (both timeout arms, error text only).
- `packages/kuru-platform/src/windows/process.rs` (read-only accessor; audited
  unsafe module, no new dependency, windows-sys features already enabled).
- `packages/kuru-delivery/tests` and `src/advisory.rs`, `src/coverage.rs` test
  helpers; `apps/kuru-tui/tests/support/terminal.rs`, `tests/terminal.rs`, and
  the install tests `embedded_runtime.rs` and `windows_cli.rs`.
- Shipped `kuru` reaches the `command.rs` arm on a Windows source build, so its
  timeout error text gains the snapshot.
- No dependency, deadline, lockfile or CI configuration changes.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
