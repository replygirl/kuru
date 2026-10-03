# Proposal

## Why

`store::creation_template::open_tests::failed_copy_after_the_build_fails_the_open_without_a_cold_retry`
failed on PR #184 (run 37092933168, job 111117083681, ubuntu-24.04-arm native
memory partition 2) with `verdict: an open whose copy failed succeeded`, and
once locally on macOS during the D2 baseline. A new project's open on an empty
template root takes the template key's shared lock, finds no template,
releases that lock by closing its handle, and tries the exclusive lock once
without waiting (`create_in`). The key lock is a `flock` on an open file
description; a child that any other thread of the process is spawning at that
moment holds a duplicate of every open description between fork and exec (the
lock file is opened `O_CLOEXEC`, so only until exec). Closing one descriptor
does not end a `flock` while a duplicate remains, so the exclusive try met a
busy lock that no Kuru holder owned and the open went cold. In the product the
same race costs a full cold build on a fresh project where the template was
available, and skips the best-effort quarantine after a verdict.

The template key lock was the one advisory lock in `kuru-memory` still
released by close alone: the project service lock (`ServiceLock::release`) and
the test template lock (`Held::drop`) already unlock explicitly for this
reason.

## What Changes

- The template key lock is held as a `KeyLock` that releases by an explicit
  `unlock()` before its handle closes, on every path: the upgrade after an
  inspection, copies on blocking threads (including an unwinding one),
  quarantines, a failed identity check and early returns.
- The key lock's time as a template build engine's reap guard is covered too:
  a failed build releases it explicitly, and the owner-drop reaper releases
  its reap guard (startup lock or key lock) explicitly after the reap, never
  before.
- The non-waiting upgrade and the designed `Busy` cold fallback are unchanged:
  a busy lock still means another Kuru holder, and no new wait or bound is
  added.
- Test hygiene: an open-path test that expects a failed open closes an open
  that unexpectedly succeeded before failing, so the failure names the
  unexpected success instead of an unreaped store.

## Capabilities

### New Capabilities

### Modified Capabilities

None. The `versioned-memory` requirement already sends an opener cold only
when "the key lock is held by another process"; the implementation let a
transient duplicate of this process's released lock count as one.

## Impact

- `packages/kuru-memory/src/files.rs`: `release_lock`.
- `packages/kuru-memory/src/store/creation_template.rs`: `KeyLock`,
  `try_key_lock`, `build`, `copy_blocking`.
- `packages/kuru-memory/src/store/stage_worker.rs`: failed build's key lock.
- `packages/kuru-memory/src/server.rs`: owner-drop reaper's reap guard.
- Tests: `creation_template/{hooks,tests,open_tests}.rs`.
- Docs: `docs/memory.md`, `docs/development.md`.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
