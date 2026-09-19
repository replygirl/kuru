## Context

`plan-flake-windows-installer.md` (read-only investigation, no code changed)
establishes: the stall is always immediately after
`bootstrap_inventory_tests::bootstrap_inventory_is_direct_and_bounded`
passes, at the very first `powershell.exe` invocation; both
`stdout_eof`/`stderr_eof` are `false` and both prefixes are empty after the
full 100 s window; `install.ps1`'s first executable statement is an
unconditional (under `-Verbose`, always passed) `[Console]::Error.WriteLine`
+ `Flush()` before `$ErrorActionPreference` is even set, so zero stderr
bytes means PowerShell never reached line 9 of the script; and `elapsed` in
`output_with_limit_and_timeout`'s timeout error is measured from
`Instant::now()` taken *after* `spawn().await` already returned
successfully, so the stall is inside the already-created process, before it
runs any of its own script — not in `CreateProcessW`/spawn. PSModulePath
inheritance is explicitly ruled out: `env_clear()` wipes the child
environment and the launch site never sets `PSModulePath`, so Windows
`CreateProcessW` with an explicit environment block never merges in a
parent value. The root cause is therefore PowerShell 5.1 engine/host cold
start (loading `System.Management.Automation.dll`,
`types.ps1xml`/`format.ps1xml`, building the initial runspace) — work that
happens before any user script line runs and that `-NoProfile`/
`-NonInteractive` do not skip — most plausibly Defender scanning
`powershell.exe`'s assemblies and/or the freshly written packaged `kuru.exe`
on first touch, or contention from the 4 concurrent Windows coverage shards.
Neither is directly provable from the existing evidence (no Defender
operational log or process sampling was captured during any of the three
failures), which is what the second half of this change addresses.

## Goals / Non-Goals

**Goals:**
- Stop the install step's fixed 100 s bound from having to also absorb
  engine cold start, without changing that bound's value or what it
  measures for the install itself.
- Make the next occurrence (of this or a different Windows stall)
  attributable from its own error text, without a retry, a log, or an
  external capture step.

**Non-Goals:**
- Explain or fix Defender/contention itself — neither is controllable from
  inside this test binary, and the diagnosis explicitly says no fix should
  be applied on unproven-cause evidence beyond adding observability.
- Change `install.ps1`, the installed executable, or any assertion the
  install step makes.
- Add retry, backoff, or widen `COMMAND_TIMEOUT`.

## Decisions

- **Warm-up is a separate, earlier process invocation, not a longer install
  bound.** Each `powershell.exe` invocation is a new process, so warming up
  "the engine" only helps the next invocation to the extent the underlying
  cost is cacheable across processes (OS page cache for `$PSHOME`
  assemblies, and/or Defender's per-file scan verdict cache) — which is
  exactly the two candidate causes the plan names. A longer single bound
  would instead let a genuinely-slow cold start continue to compete with
  the install script's own work, and would widen what `COMMAND_TIMEOUT`
  is asked to cover, which the task's own non-goals forbid.
- **The warm-up script is a trivial inline `-Command`, not `-File
  install.ps1` with a flag.** Cold start (assembly load, runspace
  construction) precedes any script line regardless of `-Command` vs.
  `-File`; using `-Command` avoids writing a second temp script and keeps
  the warm-up unambiguously about the engine, not about any `install.ps1`
  code path.
- **The warm-up bound (`ENGINE_WARM_UP_TIMEOUT`) reuses the same 100 s
  magnitude as `COMMAND_TIMEOUT`, as its own named constant.** The value is
  not "the install bound" reused — the install invocation still gets a
  fresh `COMMAND_TIMEOUT` window afterward — but 100 s is the figure this
  codebase already treats as generous for a far heavier script (32/35 of
  the sampled runs complete the whole install, including cold start, inside
  it), so it is at least as generous for a trivial script's cold start
  alone, while remaining bounded per the non-goals.
- **Diagnostic sampling uses a duplicated handle with narrower rights than
  `duplicate_process_handle`, not that handle itself.** `GetProcessMemoryInfo`
  requires `PROCESS_VM_READ` in addition to
  `PROCESS_QUERY_LIMITED_INFORMATION`, which the existing wait/query
  duplicate deliberately does not carry (its doc comment: "cannot confer
  arbitrary PID authority"). Broadening that general-purpose duplicate for
  every caller was rejected in favor of a new
  `duplicate_diagnostic_handle`, scoped to exactly query + memory-read
  rights (still no terminate, suspend, or write authority), used only by
  the sampler.
- **Sampling runs as a spawned task sampling a duplicated handle, not
  inline in the timed future.** `NativeChild::wait` needs `&mut child`;
  sampling concurrently with the existing `tokio::try_join!`/`wait` future
  would conflict with that borrow. A duplicated handle removes the
  conflict and lets the sampler be `abort()`-ed unconditionally right after
  the timed future settles (success or failure), so nothing outlives the
  call and the success path is unchanged. A sample failure (e.g. denied
  query rights) is recorded as text in the trace, never surfaced as an
  error — the feature is diagnostics-only.

## Risks / Trade-offs

- [Warm-up adds one more `powershell.exe` spawn to every Windows
  `install_packaged` run, including the 91% that were already comfortably
  inside the bound] → Mitigation: it runs before the timed step and its own
  bound only affects failure attribution, not the success-path wall clock
  budget already spent per run; the existing 91%-success baseline shows a
  cold start is normally fast, so warm-up itself should normally be fast
  too.
- [The 10 s sample interval and `abort()`-based teardown are best-effort:
  one extra sample can land in the trace after the timed future settles but
  before `abort()` takes effect] → Mitigation: the trace is diagnostic text
  only, never consulted for control flow or read on the success path, so an
  extra or missing sample has no correctness impact.
- [This does not prove which of Defender vs. contention is the cause, only
  gives CPU-time/working-set evidence for the next occurrence] → Mitigation:
  that is the explicit, bounded goal recorded in the plan; a definitive
  cause requires infrastructure the test binary cannot reach (Defender
  operational log, runner-level process sampling).

## Operational surface

No new bind address, container/runner topology, secret, or pinned binary
version/arch. Both changes run inside the existing `native-tests
(windows-2025)` job's already-selected stock PowerShell 5.1 and the
existing Rust toolchain; no new tool, service, or CI step is added. The
warm-up step only changes the shape of work already performed by that job
(one more short-lived local process), and the sampler only reads
already-spawned children's own process-time/memory counters through Win32
APIs already reachable from `kuru-platform`'s existing dependency surface
(no new crate).
