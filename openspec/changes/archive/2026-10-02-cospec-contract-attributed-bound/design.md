# Design

## Context

Two windows-latest coverage failures of the cospec contract test carry no
diagnostic, so the stall cannot be classified (plan
`tmp/roadmap/windows-families-plan-2026-10-02-cospec-contract.md`, sections 3
and 4: first execution of the 66 MB Bun executable, Bun startup or I/O, our
native pipe/Job reporting, or a later call's cold openspec extraction). The
outer `tokio::time::timeout` in the helper hid both the call and the Windows
boundary's own timeout diagnostics.

## Goals / Non-Goals

**Goals:** name the call, phase and samples on the next occurrence; record a
per-call baseline from green runs; move first execution of the installed
executable out of the measured calls.

**Non-Goals:** classifying or fixing the cause (that waits for evidence);
changing the 30 s value; retries or sleeps; pre-warming the fixture's private
`XDG_CACHE_HOME` (that would only move the extraction cost); the release
workflow's Ubuntu `tests` job, which has no occurrence.

## Decisions

- Use `kuru_delivery::command::output` rather than the outer timeout. On Unix
  it is the same `tokio::time::timeout`; on Windows it is
  `output_with_timeout` with the same bound plus diagnostics. Rejected: keeping
  the outer timeout around the 180 s default, which discards them.
- `#[track_caller]` sits on synchronous consumers of a returned `Call`, since
  it has no effect on an `async fn` on stable Rust.
- Per-call times go to the raw `io::stderr()` handle, not `eprintln!`: libtest
  captures the print macros and shows them only on failure, and the coverage
  shard runs executables with `--exact` but no `--nocapture` while inheriting
  their stderr. Rejected: changing shard flags for every test, or carrying the
  times only in a failure message (no green baseline).
- First execution belongs to the step that installs cospec: the shard job's
  mise-action step. The new step immediately follows it with `time`, bounded by
  the job limit. A step-level `timeout-minutes` was rejected because the guard
  requires the job limit to be the shard's only time limit, from which the
  inner deadline is derived.

## Operational surface

CI execution only: the native-tests `shard` job on every OS in its matrix
(ubuntu, macOS, windows-latest, windows-11-arm) gains one step running the
already-installed pinned cospec (`github:aligned-team/cospec` from `mise.toml`,
Windows x64 asset on both Windows architectures) once with `--version`. No
runner, container, secret, listener, bind address, connection limit, job
topology or tool pin changes.

## Risks / Trade-offs

- [The setup step itself stalls on first execution] → It fails the job under
  the job limit with the step named, which is itself a classification (C1); no
  retry is added.
- [A Windows timeout now surfaces up to about 5-10 s after the 30 s bound,
  after explicit Job termination and pipe joins] → Failure path only; the bound
  for each call is unchanged.
- [Ten extra stderr lines per run in the coverage log] → Small, and they are
  the baseline the plan asks for.
