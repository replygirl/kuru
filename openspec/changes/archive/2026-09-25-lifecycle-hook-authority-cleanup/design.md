## Context

The archived `lifecycle-hooks` change left these root causes open:

- **Name rewrite.** `PreToolValue { name, arguments }` accepted any tool name, and the runtime checked only its size. Deliberation dispatches every call to `cognitive_call`, which contains an `a2a_send` arm, so a hook-rewritten name could reach a tool the phase never offered.
- **Detached cleanup.** `invoke()` runs each hook on a dedicated thread so that cleanup survives when the caller drops. On cancellation, nothing tracked that thread, so harness shutdown could return during a reap.
- **Unbounded drain.** After `wait_for_exit`, `stdout.await` and `stderr.await` had no deadline.
- **Wall-clock tests.** The budget test relied on the wall clock: `sleep 0.1` against a 260 ms aggregate budget. The Windows test raised `timeout_ms` to 120 000 in place of a warm-up.

## Goals / Non-Goals

**Goals:**
- Q5 literal authority: pre-tool rewrites change arguments only.
- The offered tool set is revalidated for every final call.
- Cleanup is awaited and bounded.
- Hook records are truthful.
- Tests are deterministic.

**Non-Goals:**
- The Windows installer "strict mode ready" stall. It is owned elsewhere, and its diagnostic markers were split out of this branch.
- Changing product hook defaults.
- Adding a per-hook environment allowlist.

## Decisions

- **Rewrite checks.** The connector rejects any rewrite at each hop whose name differs from the incoming payload's name. Rejected alternative: allowing name rewrite plus offered-set checks, because Q5 grants rewriting of proposed arguments only.
- **Offered-set admission.** `run_pre_tool_hooks` receives an `OfferedTools` value, and a refused call is settled before hooks or dispatch.
  - `Exact(cognition_tools())` in deliberation and `Exact(dream_tool())` in a dream: the runtime dispatches every such call itself, so any name outside the set is refused.
  - `Speaking(available)` in the speaker phase: runtime-dispatched cognitive names must be offered; other names go to the ToolHost, which permission-evaluates the exact final call and keeps its typed refusals (for example, a denied tool hidden from the catalog).

  The check covers model-proposed calls too, because the hook path only made an existing gap deterministic.
- **Worker tracking.** `HookHost` owns an `Arc` watch channel of in-flight workers and an `uncertain` flag. A worker releases its slot only after cleanup, reap and reader completion, all under one cleanup deadline.
  - `HookHost::quiesce()` waits for zero in-flight workers under a bound derived from that deadline and reports uncertainty.
  - Three call sites await it: the turn result, the dream result and `ToolHost::shutdown`.
  - Rejected alternative: awaiting at every hook call site. It duplicates code, and a dropped caller future cannot await anything anyway.
- **Bounded drain.** The post-exit readers run under `timeout_at(min(deadline, now + CLEANUP))`. On timeout, the reader tasks are aborted, their pipes are dropped, and the hook fails closed.
- **Budget clock.** `HookBudget` takes an injectable clock that both `claim` and lease drop use. Unit tests drive synthetic instants. The real-process test uses `started-*` markers and a test-controlled release file, with no timing assertions.
- **Pre-turn provenance.** Q5 says "a rewrite changes only the current provider view."
  - Storing the original input in place of the rewrite would reintroduce the pre-rewrite text into later requests of the same turn, through the speaker's own private history. That would defeat redaction hooks.
  - Kuru therefore keeps the rewritten current input and places a `kuru-hook` provenance record (`event: pre_turn`, `outcome: rewritten`, identities, no original text) immediately before it. The record is persisted together with the input and visible to the provider as hook output.
  - The original remains in the public transcript.
- **Visible suppression.** A suppressed `HookHost` keeps the configured chains for reporting only. Each `run_*` call returns the unchanged value with one `suppressed` observation per configured hook, and runs nothing.
- **Windows environment.** The finite hook environment adds an inherited `PSModulePath` unless the resolved executable is the system `WindowsPowerShell\v1.0\powershell.exe`. The ToolHost `$PSHOME` bootstrap stays ToolHost-only.
- **Windows warm-up.** A trivial hook runs through the real `HookHost` path once per test process, under a labelled outer bound. Timed hooks then use the 5 000 ms product default, with aggregate `n × timeout_ms`. Observation waits derive from `timeout_ms`.

## Risks / Trade-offs

- **Stricter call admission.** Models that call tools not offered for their phase now get a refusal where they previously got a `cognitive_call` "unknown tool" error. The behavior is equivalent, but the message changes.
- **Real users on cold PowerShell.** A cold stock PowerShell 5.1 hook that uses cmdlets may still exceed the 5 s default for real users. That is a product question, recorded as an open follow-up. The tests no longer hide it.
- **Unverified Windows code.** The Windows paths compile-check locally at most. Native Windows behavior remains a CI gate.
