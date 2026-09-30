# Proposal

## Why

Phase 1 of the startup regression needs open times for the release binary a user runs on ubuntu-latest, measured in CI; no record has any, and nothing times the owner's open from inside a release build. This change adds a report-only measurement, from outside the binary, inside `ci.yml`, shaped as the first form of the open-time budget check that phase 2 will turn into a gate. macOS numbers come from one-off local runs of the same harness, not from a CI job; Windows open time is not measured or documented here.

This is also P0 of the store creation work (`tmp/roadmap/store-creation-design-machine-cache-2026-09-29.md` section 11): the acceptance instrument that must exist, with a `new-project` case and a warm-engine baseline, before the template-cache PRs land, so their before/after engine-start counts have something to compare against. Section 12 (M3) names this job and a local macOS run as the measurement points for every PR in that split.

## What Changes

- `packages/kuru-delivery/src/open_time.rs` and `src/open_time/`: an `open-time` subcommand of the maintainer `kuru-delivery` tool, behind the unshipped `tooling` feature. It drives a release `kuru` in isolated scratch roots through four cases (first launch ever, a second, different project created in the same scratch root right after the first so its engine cache is already warm, first open of an existing project with no live owner, warm reopen inside the owner's idle window), timestamps its stderr progress lines, and observes processes (sysinfo) and directory listings under the scratch root, never opening a file and never entering the install stage, store staging, interrupted or template-cache build/stage directories the product renames. It cuts each open at ordered milestones into stages that partition it, reports nesting spans as totals with their parts, and states the recorded sampling bracket as the error bound. Before and after each run it takes a census of what can accumulate (this user's Kuru and Dolt processes, open handles and listening ports of the scratch root's processes, staging and interrupted directories) and records the owner's retirement time. A control mode lists no files, and a ramp mode runs consecutive opens without retirement waits. It writes one `kuru.open-time.v2` JSON record per run and a summary table. It never compares a time with a budget.
- The harness reads the end of an open from either of two signals: the legacy `Memory: ready.`/`Memory: waiting for project ownership…` stderr lines, or, when a run's binary supports it, the `kuru-open-marker v1 <event> <monotonic_ns>` stderr lines (`open-start`, `waiting-ownership`, `ready`) that a separate, not-yet-merged change emits when `KURU_OPEN_MARKERS=1` is set. The harness sets that variable on every run it launches so both old and new binaries measure; the record names which signal ended the open.
- `packages/kuru-delivery/tests/fixtures/delivery.rs`, `tests/open_time.rs`: a live-subprocess fixture that stands in for `kuru` and drives the harness end to end; extended to emit both signals (and neither, for the failed-open case) so the harness's reading of each is tested against a live process.
- `packages/kuru-delivery/mise.toml`: `measure:open-time` task.
- `Cargo.toml`, `packages/kuru-delivery/Cargo.toml`, `Cargo.lock`: `sysinfo =0.39.6` (feature `system` only), optional under `tooling`. The shipping `kuru` dependency tree is unchanged on every target.
- `.github/workflows/native-tests.yml`: the ubuntu-latest install job publishes its installed release binary as an artifact (step-level `continue-on-error`); the other install jobs (macOS, Windows, Windows on Arm) do not.
- `.github/workflows/ci.yml`: an `open-time` job on ubuntu-latest only that downloads that binary, runs the task as four series (main, control without file observation, a coarse 200 ms sampling period, and a ramp without retirement waits) and uploads the records. Job-level `continue-on-error`; not in `ci-gate.needs`; no `workflow_dispatch`, no new workflow file.
- `docs/development.md`: what the job measures, that it is report-only, the four cases and their expected engine-start counts, both readiness signals and the `KURU_OPEN_MARKERS` switch, the stages and their error bounds, the census and modes, which build profile the packaged embedded-runtime test runs in coverage partitions, how to read it, and the phase 2 step that would make it a gate.

## Impact

One new non-gating job on ubuntu-latest in `ci.yml`, which starts after `native-tests` finishes. One extra upload step in the ubuntu-latest install job; that upload runs inside the gating install job, so its time is on `ci-gate`'s critical path. No product code, no product dependency, no secrets, no permissions, no required checks and no action pins change. The job builds only `kuru-delivery`, which does not depend on `kuru-memory`, so it needs no engine bundle input. This change adds no marker emission itself: reading `kuru-open-marker` lines is additive and the harness keeps working, on the legacy signal only, against a binary from before the marker-emitting change merges.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
