# Proposal

## Why

The bootstrap's `bounded()` now stops a producer at its cap with a kernel
file-size limit (`ulimit -f`), reporting status 153 (128 + SIGXFSZ) as
"exceeds size limit". Two side effects of that limit were left open. First,
the producer inherits the caller's core-dump limit: on Linux with
`ulimit -c unlimited` and a plain `core_pattern`, an over-cap producer killed
by SIGXFSZ leaves a `core` file in the installer's working directory, a new
user-visible effect of an oversized download. Second, the "exceeds size limit"
text depends on the caller's signal disposition: when an ancestor ignores
SIGXFSZ (a non-interactive shell cannot reset an ignored signal), the write past
the cap fails with EFBIG instead, the producer exits 1 with a cap-sized output,
and the bootstrap reports "failed (producer 1)" although the tested contract is
"exceeds size limit".

## What Changes

- The producer group sets `ulimit -c 0` before its file-size limit, so a
  producer stopped at the cap cannot write a core dump.
- Status 153 stays "exceeds size limit". For any other nonzero producer status,
  a stage output already at least as large as the cap is also reported as
  "exceeds size limit"; otherwise the failure keeps "failed (producer N)". The
  153 rule is not replaced by a size rule, because a sparse extraction trips
  SIGXFSZ by seeking past the cap with a near-empty output file.
- `wc` returns to the bootstrap's required tools.
- `docs/install.md` notes that the per-step limit also applies to the
  installer's own output when it is appended to a regular file.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

- `packages/kuru-delivery/support/install.sh`: the producer group's limits,
  the failure classification in `bounded()`, the required-tool list.
- `packages/kuru-delivery/tests/bootstrap_install.rs`: regression tests that
  launch the bootstrap with SIGXFSZ ignored and that read the producer's
  core-dump hard limit through the fixture `curl`.
- `docs/install.md`: one sentence on redirecting installer output.
- Residuals (recorded, not changed): with SIGXFSZ ignored and an inherited hard
  file-size limit below a cap, the size fallback compares against the larger
  cap and reports "failed (producer N)"; with SIGXFSZ ignored, a sparse
  extraction past the cap also reports "failed (producer N)". Both still fail.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
