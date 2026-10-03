# Proposal

## Why

`apps/kuru-tui/tests/terminal.rs`
`real_pty_first_launch_shows_the_creating_sentence_while_the_template_builds`
failed on PR #195 (run 37131855148, job 111228490168, ubuntu-latest coverage
partition 5). The only failure the log shows is the `ServiceCleanup` Drop
panic (`apps/kuru-tui/tests/support/memory.rs:200`, the branch that runs on a
non-panicking thread): "idle managed owner did not retire within 10 seconds;
maintenance waiting for the owner lock for 53ms (this lock's wait 10001ms);
requests without a live endpoint=99; ...". The fixture spent its whole flat
10 s waiting for the owner lock while no endpoint was live.

Whether that owner had published is inferred, not measured. The test returns
`Result` and never calls `sandbox.release(outcome)`, so its locals drop before
its value returns and the Drop panic discards an `Err` exactly as it would an
`Ok`: the log cannot show whether the body reached its composer-frame
assertion. A fast `Err` is possible: `Terminal::wait` bails at once when the
kuru child has exited, and the owner runs in its own process group
(`service.rs` `process_group(0)`), so it outlives a starter that died before
publication and holds its lock with no endpoint, producing the same 99
no-endpoint readings. Verification 1.1 below reproduced the identical old text
(`requests without a live endpoint=100`) from an owner held before
publication. The most likely reading is a published owner closing after the
TUI exited (plan options B/C), but a pre-publish owner (option A) is not
excluded by this occurrence. The follow-on that makes the next occurrence
decidable is releasing the terminal tests' sandbox through
`sandbox.release(outcome)` (plan 5b), so a cleanup failure attaches to the
test's own outcome instead of discarding it.

Two defects meet here. First, the expiry text names only the fixture's own
step: an owner still opening, which never published, and an owner closing after
a publish read identically (no live endpoint at every request, owner lock held),
so neither the readiness family (plan option A) nor a slow close (B/C) can be
told apart from the failure. Second, the 10 s is a fixture guess that no product
budget derives, policing the remainder of a close whose own budget is
`close_budget()` (32 s). The lead's rule for the second is to wait on the owner's
lock release under the fixture's existing outer backstop, without raising or
guessing a bound; `ServiceCleanup` and the mise acceptance fixture join their
cleanup threads with no backstop at all, so that replacement is blocked and is
reported, not made.

## What Changes

- `test_support::retire_idle_service`'s expiry error keeps its bound and its
  trace, replaces its lead "idle managed owner did not retire" with the neutral
  "managed owner retirement did not complete within 10 seconds" (an owner still
  opening is not an idle owner that refused), and adds the state of the owner
  it waited behind, read at expiry from the owner's own records with reads only
  (no lock, no failure path): `owner published; its endpoint record is still
  present`; `owner still opening; last stage = <stage>; no endpoint record
  present`; `owner open failed before its
  starter attached; closing its store; reason = ...`; `owner closing; last phase
  = endpoint and activity records retired (store close, Dolt reap or owner-lock
  release outstanding)`; or `owner state unreadable: ...`. (The 10 s prefix
  is superseded by the Decision below: the close budget's and the startup
  deadline's expiries carry these owner-state readings instead.)
- A test-support-only `service::activity::inspect` reads the open-activity
  record under its name whatever its tag, for that diagnostic only. It decides
  nothing; the record still grants no authority.
- (Superseded by the Decision below.) The flat 10 s is unchanged. The missing outer backstop is documented on
  `retire_idle_service`, with the candidate shapes recorded for the lead in the
  handoff notes. No product close, election, lock or record behaviour changes;
  no retry, sleep or bound is added.

### Not delivered

The task allowed close-phase stamps through the existing seams if none
existed. None exist, and none were added: a stamp readable during the close is
a new write inside the product close, which plan §6 step 2 leaves to the lead.
So the diagnostic does not print the time elapsed since the close began, and
it does not split the close into store close, Dolt reap and owner-lock
release; "owner closing; last phase = endpoint and activity records retired"
covers that whole span. Those stamps would also enable the backstop shape this
change cannot adopt without them: `close_budget()` (32 s), counted from a
close-start stamp the owner records, around an event wait on the owner's lock
release.

## Decision: the bound (option b, amended 2026-10-03)

The lead decided option (b) after the BLOCKED report above, and this change
now also lands it (amended in place; no new change):

- `retire_idle_service` asks through the maintenance acquisition as before,
  keeping the active-client refusal loop (retry with a 20 ms backoff while
  the owner refuses with attached clients). That asking phase has no
  fixture-only bound: it is bounded by one deadline carried across the
  refusal retries, `memory.startup_timeout_secs` (30 s by default) from its
  first request, the deadline each `acquire_maintenance_permit_traced`
  attempt already enforces on its start-lock wait, owner response and
  owner-lock wait (an attempt also ends Busy after 10 replies; only the
  fixture's refusal retry was otherwise unbounded). It covers only an owner
  that has not shown it is closing (a client still attached, the start lock
  held, or no reply yet). Its expiry, and an attempt failing at that same
  deadline, names the deadline, the time since the first request, the owner
  state, the trace and the refusal count; product behaviour is unchanged. An
  interim fix commit (818294f3) kept the former flat 10 s for this phase as
  `ANSWERING_OWNER_BOUND`; review rejected that self-approved retention, since
  the lead's decision replaced the flat 10 s, and 055d2818 removed it.
- At the first retirement reply that shows the owner closing (no live
  endpoint or a connection the owner closed unanswered, as the lead named, and
  also an accepted retirement request, which is the same close and would
  otherwise still be policed by the asking deadline), it drops the acquisition and waits
  on the owner lock's release event (`await_owner_release`, no deadline by
  design) under one backstop: `server::close_budget()` (`CLOSE_GRACE` 8 s +
  `KILL_GRACE` 3 s + `SUPERVISOR_REAP_ALLOWANCE` 13 s + `CLOSE_GRACE` 8 s =
  32 s, the product's own derived close budget) counted from that first
  reading, a lower bound of the close's age. The reading is stamped on the
  existing `MaintenanceTrace` (test-support only field and signal); product
  messages and behaviour are unchanged.
- The lock wait cannot be cancelled, so it runs on its own thread with its own
  current-thread runtime, as the terminal tests' `await_owner_exit` does; the
  async side waits on a oneshot under `timeout_at(backstop)`. At the backstop
  the retirement fails and the blocked thread ends with the process (or once
  the owner lets go). No retry and no sleep are in the release path.
- After the release, the maintenance permit is taken and dropped once more
  under the same backstop, so the completion proof is unchanged.
- The backstop's expiry names the budget, the time since the first closing
  reading and which reading it was, the owner's state from #197's readings
  (published / still opening at stage X / failed open closing / closing with
  records retired / unreadable), the trace and the refusal count.
- Because the bound lives inside `retire_idle_service`, every caller is
  bounded without an outer backstop: `ServiceCleanup::retire` (the PTY and
  ConPTY terminal tests) and mise acceptance `retire_blocking` keep their
  thread-and-join shape, whose join now ends by that backstop, and the
  runtime callers (`dream.rs`, `accounting_tests.rs`) are unchanged.

Not in this change: option (c), deriving the product maintenance permit's
own election deadline (`startup_timeout_secs`, 30 s by default, shorter than
the 32 s close budget) from `close_budget()`. It changes product maintenance
behaviour and is a separate PR. Close-phase stamps (see "Not delivered") are
still not added.

## Capabilities

### New Capabilities

### Modified Capabilities

None. Only test support changes; the owner's close, election and records are
unchanged.

## Impact

- `packages/kuru-memory/src/test_support.rs`: `retire_idle_service` expiry text,
  new `owner_state`, the existing elapsed-bound test's assertion; (decision b)
  the event wait on the owner lock release under `close_budget()`.
- `packages/kuru-memory/src/service.rs`: (decision b) the trace's test-support
  first-closing-reading stamp and signal.
- `apps/kuru-tui/tests/support/memory.rs`,
  `packages/kuru-delivery/tests/support/mise_acceptance.rs`: (decision b) doc
  comments only; their joins are bounded by the retirement's backstop.
- `packages/kuru-memory/src/service/activity.rs`: test-support `inspect` and
  `describe_stage`; two regression tests.
- Docs: `docs/development.md` (fixture retirement).

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
