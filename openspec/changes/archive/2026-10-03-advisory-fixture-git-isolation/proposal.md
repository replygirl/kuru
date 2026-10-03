# Proposal

## Why

The advisory fixtures' own `git commit` has repeatedly hit its 10 s tool
timeout on Windows runners (most recently PR #192 run 37125916805, job
111211190643, `src/advisory.rs:542`). The Job held only two `git.exe`
processes, and the timeout message could not name the child or the phase.
Neither fixture helper disables commit auto-maintenance, and the integration
helper inherits the runner's system and global Git configuration. Each test
also rebuilds its repositories with six to twenty-seven fixture Git spawns.

## What Changes

- `packages/kuru-delivery/tests/support/fixture_git.rs` (new) adds one shared
  fixture-Git builder. It is included by the `src/advisory.rs` test module,
  `tests/advisory.rs` and the scanner fixture `tests/fixtures/delivery.rs`.
  - Environment: it removes inherited config injection, `GIT_ASKPASS` and
    `SSH_ASKPASS`, and sets `GIT_CONFIG_NOSYSTEM=1`, an empty
    `GIT_CONFIG_GLOBAL`, a temporary `HOME` and `XDG_CONFIG_HOME`,
    `GIT_TERMINAL_PROMPT=0` and `GCM_INTERACTIVE=never`.
  - Flags: an empty `core.hooksPath`, `maintenance.auto=false`, `gc.auto=0`,
    `core.fsmonitor=false`, an empty `credential.helper`,
    `commit.gpgsign=false`, `tag.gpgsign=false` and `core.autocrlf=false`.
  - Every fixture call writes `GIT_TRACE2_EVENT` to its own file in the
    fixture's temporary directory. A failure or timeout panic carries the
    trace's last 40 lines.
- The unit helper keeps its hostile `GIT_DIR`/`GIT_CONFIG_COUNT` setup and
  production `GitEnvironment` sanitization. It applies the builder before and
  after them, so production sanitization stays under test.
- `kuru-platform/src/windows/process.rs` reports each Job member's full image
  path rather than its file name, so a diagnostic distinguishes
  `Git\cmd\git.exe` from `mingw64\bin\git.exe`. Its Windows tests
  (`tests/windows_process.rs`, kuru-delivery `tests/windows_command_capture.rs`)
  assert an absolute path that canonicalizes to the fixture executable.
- The advisory fixture repositories are built once per test binary
  (`tokio::sync::OnceCell`), and each test copies the directory tree, as #116
  did for memory stores. Variants such as attached, stale, wrong-origin and
  executable-config are templates too. Per-test fixture Git spawns fall to
  zero. Every template build asserts that its Trace2 files show no
  `child_start`.
- The refresh test's database fixture no longer performs its own `fetch`. A
  local fetch always starts `git-upload-pack`, `pack-objects` and
  `unpack-objects` children, which a zero-child fixture cannot contain. The
  stale checkout is now its own detached root commit, with origin `../source`
  copied with it. The production refresh still fetches it.
- Deterministic tests, native on every platform:
  - T1: the fixture sequence writes a Trace2 `start` per call and no
    `child_start`.
  - T2: in a child test process, a hostile system/global config and
    `GIT_CONFIG_COUNT` injection set marker hooks, a credential helper,
    fsmonitor, maintenance, gc and signing. The test asserts that no marker is
    written and no child is started. A hooks-only control proves the hostile
    configuration is live.
  - T3: under the builder, `git config --list --show-origin` lists only
    `command line:` and the fixture's own `.git/config`.
  - T4: the fast-failing launch panic names the Trace2 tail field.
- Not changed: the 10 s fixture bound, the 60 s production bound, retries and
  sleeps, and production `GitEnvironment` sanitization.

## Impact

- Test-only, apart from Windows diagnostic text, which now shows full image
  paths. Fewer Git processes run per delivery test binary.
- The Windows full-path assertions run only in native Windows CI.
- The `OnceCell` templates are never dropped. The unit-test template directory
  is left in system temp once per binary run, and the integration templates
  live under Cargo's target tmp directory.
- Follow-on, owned separately: production advisory `fetch`
  (`src/advisory.rs` `refresh_with_origin`) also triggers auto-maintenance
  under its 60 s bound. Whether owned advisory Git should pass
  `maintenance.auto=false` is a product decision outside this change.
