# Proposal

## Why

`apps/kuru-tui/tests/terminal.rs`
`real_pty_first_launch_shows_the_creating_sentence_while_the_template_builds`
failed on PR #195 (run 37131855148, job 111228490168, ubuntu-latest coverage
partition 5) after every one of its assertions passed, including the composer
frame that the TUI draws only once its managed memory open has attached to the
owner's published endpoint. The `ServiceCleanup` Drop then panicked: "idle
managed owner did not retire within 10 seconds; maintenance waiting for the
owner lock for 53ms (this lock's wait 10001ms); requests without a live
endpoint=99; ...". The owner had published and, once the TUI (its starter and
only client) exited, began its own close; the fixture spent its whole flat 10 s
waiting for the owner lock behind that close.

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

- `test_support::retire_idle_service`'s expiry error keeps its text and its
  bound and adds the state of the owner it waited behind, read at expiry from
  the owner's own records with reads only (no lock, no failure path):
  `owner published; its endpoint record is still present`; `owner still opening;
  last stage = <stage>; no endpoint published`; `owner open failed before its
  starter attached; closing its store; reason = ...`; `owner closing; last phase
  = endpoint and activity records retired (store close, Dolt reap or owner-lock
  release outstanding)`; or `owner state unreadable: ...`.
- A test-support-only `service::activity::inspect` reads the open-activity
  record under its name whatever its tag, for that diagnostic only. It decides
  nothing; the record still grants no authority.
- The flat 10 s is unchanged. The missing outer backstop is documented on
  `retire_idle_service`, with the candidate shapes recorded for the lead in the
  handoff notes. No product close, election, lock or record behaviour changes;
  no retry, sleep or bound is added.

## Capabilities

### New Capabilities

### Modified Capabilities

None. Only test support changes; the owner's close, election and records are
unchanged.

## Impact

- `packages/kuru-memory/src/test_support.rs`: `retire_idle_service` expiry text,
  new `owner_state`, the existing elapsed-bound test's assertion.
- `packages/kuru-memory/src/service/activity.rs`: test-support `inspect` and
  `describe_stage`; two regression tests.
- Docs: `docs/development.md` (fixture retirement).

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
