# Proposal

## Why

A vendor-side apt stall can hold a native-test partition until the job's time
limit: main run 36887289590, job 110454097537 (ubuntu-latest coverage partition
2, head b54310e0) started the "Install Ubuntu native secret-store fixture
tools" step at 15:53:11Z, `apt-get update` printed
`Get:5 https://archive.ubuntu.com/ubuntu noble-security InRelease [126 kB]` at
15:53:29Z and then nothing until the 45-minute job limit cancelled it at
16:35:56Z. apt's own fetch timeouts and retries did not end it, so the bound
must be external to apt.

## What Changes

- `.github/workflows/native-tests.yml`: the Ubuntu apt step (reached from
  `ci.yml`'s `native-tests` job through `workflow_call`) gets a step-level
  `timeout-minutes` and one bounded retry of the apt interaction only. The
  `[ -f /etc/apt/sources.list.d/ubuntu.sources ]` guard stays outside the loop;
  inside it, at most two attempts each run the literal `apt-get update` then
  `apt-get install` under coreutils `timeout <attempt budget> sudo apt-get ...`,
  keep `-o Dir::Etc::sourcelist=/etc/apt/sources.list.d/ubuntu.sources -o
  Dir::Etc::sourceparts=/dev/null`, print the attempt number and, on failure,
  apt's exit status (124 = timed out), and a second failure fails the step with
  that status. No `Acquire::Retries` stacking and no `dpkg --configure -a`
  recovery. Step `timeout-minutes` = 2 attempts x 2 fetches x attempt budget,
  rounded up to whole minutes (748 s -> 13; the 32 s left covers the guard and
  the attempt messages). Every number carries its derivation in a comment at
  the site (see Measured basis); no guessed literal.
- `packages/kuru-delivery/tests/repo_validation.rs`: the constant `FIXED_APT`
  that pins the step text for both `native-tests.yml` and `release.yml` is split
  so native-tests gets the new text and release.yml keeps the old text;
  `incident_apt_update_over_every_source_is_rejected` is updated; one new
  assertion shows that removing `-o Dir::Etc::sourceparts=/dev/null` from one
  apt-get inside the new retry loop of native-tests.yml still yields the "over
  every configured apt source" error naming that command. The validator
  (`packages/kuru-delivery/src/repo/workflows.rs`, apt rule) must still see both
  fetches in the new shape; no new validator rule.
- `packages/kuru-delivery/tests/release_workflow.rs` (found during
  implementation): `native_workflow_partitions_every_os_and_keeps_the_aggregate_fail_closed`
  collected every line of the native `shard` job that trims to
  `timeout-minutes: ` and required exactly `["45"]`, so the new step-level
  bound would fail it. Its filter now reads only the job-level key at job
  indentation (`    timeout-minutes: `); the assertion it guards
  (`KURU_COVERAGE_JOB_MINUTES` equals the job's limit) is unchanged. The only
  way to keep the diff to two files would be to drop the step-level backstop.
- Non-goals: `release.yml` has an identical step and is left unchanged (reported
  as a follow-on); no other workflow step or file, no new workflow, no retry of
  tests. This is the single sanctioned retry shape here: vendor-side (Ubuntu
  archive / Azure mirror), never a retry of tests.
- Docs: `docs/development.md` (apt rule paragraph near line 304) describes the
  validator's source restriction, not this step's timing; the restriction is
  unchanged, so no docs change is expected. No other doc mentions the step.

## Impact

Jobs: every Ubuntu partition of `native-tests (ubuntu-latest)` (and any other
Linux caller of `native-tests.yml`). Healthy runs are unaffected (attempt 1
succeeds in tens of seconds); a stall now ends within the step bound with apt's
status instead of a 45-minute job cancel. No secrets, required-check names or
job names change. The step cannot be provoked into a stall in CI, so the stall
behavior is reasoned from the shape, not observed.

## Measured basis

For the implementer's derivation comments (collected 2026-10-03, read by the
change author; statistics are measurements, readings of them are inferences).

Healthy step duration, step "Install Ubuntu native secret-store fixture tools",
`started_at` to `completed_at`, all jobs named `native-tests (ubuntu-latest) /
Coverage partition ...` in the 25 most recent successful main CI runs (8
partitions each): n = 200, all `success`; min 12 s, median 15 s, p95 27 s, max 43
s (timestamps have 1 s resolution). Slowest three: job 111292093490 (run
37153465147, 43 s), job 111313661609 (run 37160753044, 42 s), job 111296132453
(run 37154711185, 41 s). 189 of 200 jobs finished within 24 s. Sample timeline,
job 111126549651 (typical, 10 s): update fetched at +4 s, install fetched at
+6 s, dpkg triggers done at +10 s. Slowest, job 111292093490 (43 s): `update`
ended 11 s in with a 10 s gap after the last `Get:`, install fetch took about 2
s, dpkg setup and triggers took about 26 s (a 15 s trigger step).

The two healthy logs read (jobs 111292093490 and 111126549651) show every index and package served by `http://azure.archive.ubuntu.com`
(mirror list `file:/etc/apt/apt-mirrors.txt`) with no `Ign:` lines (the other 198 logs were not read). The hang log
(job 110454097537) differs: the Azure mirror answered `Ign:` for every
InRelease at 15:53:26Z (about 15 s after start, after the mirror list at
15:53:11.5Z), `Ign:` again at 15:53:27Z, then apt fell back to
`https://archive.ubuntu.com` (`Hit:`/`Get:` at 15:53:28Z to 15:53:29Z), and
stalled after the `noble-security` Get at 15:53:29.7Z. So an unhealthy
`apt-get update` can legitimately spend ~15 s in mirror fallback before
progressing (inference: the attempt budget must exceed this plus a healthy run).

apt version: the runner image `ubuntu-24.04` (runner-images 20260927.320.1) is
noble; noble ships apt 2.8.3 (manpages.ubuntu.com/manpages/noble pages state
"Provided by: apt (Version: 2.8.3)" for apt.conf(5), apt-transport-http(1) and
apt-transport-https(1)).

apt documentation, noble apt 2.8.3. Cited text: apt-transport-http(1) and
apt-transport-https(1): "The option Acquire::http::Timeout sets the timeout
timer used by the method; this value applies to the connection as well as the
data timeout" (the https page documents the same setting for `https`; its
example block shows `Timeout "10"` as an example value only). apt.conf(5),
Acquire: "Retries: Number of retries to perform. If this is non-zero APT will
retry failed files the given number of times." The noble man pages state no
default for either option. The defaults come from the 2.8.3 source (salsa.debian.org
apt-team/apt, tag 2.8.3, read by the author): `methods/basehttp.cc` constructs
`ServerState(...)` with `TimeOut(30)`, so the transport timeout default is 30 s
per wait, and `apt-pkg/acquire-item.cc` initialises `Retries(_config->FindI(
"Acquire::Retries", 3))`, so per-item retries default to 3 (inference from the
source: these are per-wait and per-item bounds, with no documented total cap for
an `apt-get` process, which is consistent with the 42-minute stall surviving
them). Hence the external `timeout` wrapper; `Acquire::Retries` is not raised
or stacked.

Added by the implementer (2026-10-03, read from the same tag 2.8.3 sources):
`apt-pkg/acquire-worker.cc` `pkgAcquire::Worker::HandleFailure` (lines
644-648) delays each transient-failure retry when `Acquire::Retries::Delay`
is true (default `true`) by `min(1 << Iter, Acquire::Retries::Delay::Maximum)`
seconds (maximum default 30), where `Iter` is 0, 1, 2 for the three default
retries: 1 + 2 + 4 = 7 s. So one item whose every attempt stalls on a single
wait is ended by apt itself after at most (1 + 3) x 30 s + 7 s = 127 s
(inference from the source: `Timeout`, `ConnectionTimedOut` and
`ConnectionRefused` are among the transient reasons it retries). The hang
log's fallback, measured: mirror list `Get:1` at 15:53:11.557Z, first Azure
`Ign:` at 15:53:26.879Z (15.3 s), first fallback `Hit:2` from
`https://archive.ubuntu.com` at 15:53:28.569Z (17.0 s). The step's per-fetch
budget is 127 s + 17 s + 43 s (the healthy whole-step maximum above, a
conservative allowance for one fetch, including the ~26 s dpkg phase) =
187 s, and its `timeout-minutes` is 2 attempts x 2 fetches x 187 s = 748 s,
rounded up to 13 minutes.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
