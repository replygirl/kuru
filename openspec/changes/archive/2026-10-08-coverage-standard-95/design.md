# Design

## Context

The downloaded native reports match checked main `a549af91`. Canonical merge
summaries require 477 additional existing covered lines on Ubuntu, 455 on macOS,
and 2,204 on Windows before allowing for meaningful test additions. Windows also
has a separate native platform gate. The existing threshold is represented by
the delivery merge constant and package-owned cargo-llvm-cov task arguments.

## Goals / Non-Goals

Use the existing ownership boundaries and fixture infrastructure to exercise
observable refusal, recovery and provider-continuation contracts. Avoid a new
testing framework or coverage metric, artificial denominator padding, unrelated
runtime refactoring, paid inference, tool upgrades, and public publication.

## Decisions

1. Keep the current line metric and native inventory. Local combined coverage,
   hosted per-OS merge and existing package coverage commands all require 95%.
   A rounded printed percentage or another OS's result cannot satisfy a failed
   exact boundary. Replacing the metric would conceal the measured gaps.
   Select the managed coverage executable directly and check its version:
   Cargo can prefer an older subcommand in `CARGO_HOME/bin` over mise's PATH.
   Local diagnosis reproduced that shadowing with ambient 0.8.7 and managed
   0.9.1. The standalone package launchers require the same explicit selection
   already used by combined coverage and native partitions.
2. Select cases from actual missed lines and the owning contracts. Use real
   native objects, isolated fake secrets and deterministic failure controls.
   Add assertions about untouched adjacent state, exact identities, typed errors
   and observed provider requests. Broad snapshot or success-only tests would
   not demonstrate the uncovered safety paths.
3. Reproduce suspected defects before correcting them. The legacy receipt's
   error flag must survive into the provider request; malformed evidence must
   not become a successful receipt. Do not alter correct scoped contracts merely
   because a neighboring branch is uncovered.
4. Coordinate one local coverage writer. Package lint and scoped tests may run
   independently after their prerequisites, but native CI proves Windows code
   and enforces every required gate before merge. Whole-workspace instrumentation
   uses its own supervisor, never an ordinary prepared supervisor snapshot.

## Operational surface

Fixtures run in the existing native Ubuntu, macOS and Windows runner processes,
without containers. Local HTTP/IPC fixtures bind only loopback or their isolated
private namespace, use ephemeral addresses and retain the owning listener.
No live provider or publication secrets are needed; provider fixtures use fake
credentials, while CI retains its existing scoped service credentials. Existing
connection, process, frame and fixture bounds apply. Rust, Dolt and tool versions
and the supported native target architectures remain pinned as shipped.

CI runner labels, exact-test inventories, instrumentation modes, private bundle
preparation, credentials, action pins and supported platform requirements remain
as shipped. The workflow changes only current coverage policy/labels and its
owned contract expectations. Full PR CI and the exact resulting main commit
must pass before completed worktrees are retired.

## Integration contract

Existing platform APIs own filesystem, process and private IPC mechanics;
existing memory fixtures own Dolt and session state. Provider and HTTP fixtures
use synthetic requests and stores. No test accesses an account credential store
or turns an allowed shell into a claimed sandbox. User-facing assertions inspect
the public operation outcome and persisted state, with bounds for diagnostics.

## Evaluation criterion

A synthetic provider must observe a failed legacy tool receipt as
`ToolResult { is_error: true }`, while valid success/missing legacy flags retain
their compatibility behavior. Malformed current receipts fail before a request.
This is deterministic tool-protocol correctness, requiring no model-behavior
tuning or paid live completions. Reasoning sidecar conflict cases check actual
private persistence and the invocation's observable failure.

## Risks / Trade-offs

- Native-only gaps require hosted execution → validate Windows source with
  cross-target lint, then require actual native fixtures and 95% gates in CI.
- Failure injection can create a cleanup leak → retain checked owners, use
  closing scopes, and assert exact adjacent state and reaping outcomes.
- Raising the gate exposes further useful gaps → expand the relevant contracts
  until every gate passes; do not reduce the requested standard.
