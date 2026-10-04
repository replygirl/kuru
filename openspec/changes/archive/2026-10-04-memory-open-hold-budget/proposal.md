# Proposal

## Why

The owner's test-support open hold (`service/activity.rs::hold`, compiled only
for `cfg(any(test, feature = "test-support"))`) waits for the holding test to
remove a marker file, and bounds that wait by `memory.startup_timeout_secs`
(30 s), a product window that has nothing to do with how long the holding test
needs to observe the stage. On a loaded runner the test saw the stage later
than 30 s after the hold began; the hold expired, creation went ahead, and the
test's precondition failed.

Measured, PR #212 CI run 37180709856, ubuntu-latest coverage partition 5 (job
111373013270, head bacb9288):
`apps/kuru-tui/tests/terminal.rs::real_pty_first_launch_shows_the_creating_sentence_while_the_template_builds`
failed after 35.8 s. The owner logged `memory open hold failed: open hold on
CreatingDatabase was not released`, and the test then failed with `the store
template cache was used before creation began`. The test body is unchanged by
#212, and #212 only lengthened `sandbox.startup_timeout`, which cannot make a
30 s hold expire sooner. Not reproduced locally; the mechanism below is read
from the code, not observed.

Mechanism (measured on origin/main 449dca9e):

- `open_owner_store` sets `hold_limit = Duration::from_secs(options.config.startup_timeout_secs)`
  (activity.rs:957) and passes it to `hold` (:988).
- `hold` (:870-921) computes one `deadline = now + limit`, shared by two waits:
  the record write (`timeout_at(deadline, feed.written.wait_for(..))`) and the
  marker loop (`while marker.exists()` with a 10 ms poll, failing with `open hold
  on {stage:?} was not released` once `now >= deadline`).
- A failed hold is only logged (`eprintln!`, :989) and the open proceeds. The
  hold therefore ends on a timer the holding test does not control, in the one
  case where the test needs it not to.

The binding team rule is that no flat wait decides an outcome: a wait ends on
its event, under a budget derived from a product or vendor value. The marker's
removal is the event and stays the event. What is wrong is the budget: it must
be the budget of the party that removes the marker, the holding test's own
observation budget (for the TUI sandbox, `startup_timeout`, derived at
`apps/kuru-tui/tests/support/terminal.rs:107-114` from the product's startup
budget), not the product startup window.

## What Changes

- **The marker carries its holder's budget.** `hold` reads the content of
  `<dir>/<Stage>.hold` once, when it finds the marker at the stage's start.
  Content that is a decimal number of milliseconds replaces `limit` for that
  stage's hold, so one deadline bounds both the record write and the marker
  wait, as today. An empty marker (or only ASCII whitespace) supplies no budget
  and the bound stays `startup_timeout_secs`, exactly as today. Any other
  content, or a number whose deadline would overflow, fails the hold through
  its existing path (`memory open hold failed: ...`, logged, open proceeds)
  with an error that names the marker; it never falls back silently. A marker
  removed between the existence check and the read counts as already released.
- **Document the format and derivation at `OPEN_HOLD_DIR_ENV`** (doc comment,
  activity.rs:65-69) and in the hook paragraph of `docs/development.md`
  (~785-790): the budget is the holding test's longest wait for the stage's
  observable effect, derived at the test from the product budgets that wait
  encloses, never a literal.
- **One deterministic test** in the activity.rs test module, on a paused
  clock, calling `hold` directly with `logged_feed(..)` and a small `limit`
  (1 s, standing in for a small `startup_timeout_secs`):
  1. a marker carrying a budget well above the limit keeps the hold pending
     past the limit and releases `Ok` when the marker is removed, before the
     budget; and a marker with unreadable content fails naming the marker;
  2. an empty marker with the same limit still fails with `was not released`
     at the limit while the marker still exists, so today's bound is unchanged.
  The test uses virtual time and the marker's removal, with no wall-clock
  window and no engine.

### Hand-off contract for callers (adopted by PR #212 after this merges)

- **File:** the existing `<dir>/<Stage>.hold`, where `<dir>` is the value of
  `OPEN_HOLD_DIR_ENV` (`kuru_memory::test_support::OPEN_HOLD_DIR_ENV`) and
  `<Stage>` is the `{stage:?}` name (for example `CreatingDatabase.hold`). No
  new file and no new environment variable.
- **Format:** the marker's content is the budget as ASCII decimal milliseconds,
  with optional surrounding whitespace such as a trailing newline: for example
  `std::fs::write(&hold, sandbox.startup_timeout.as_millis().to_string())?`.
  An empty marker (today's `std::fs::write(&hold, b"")`) means no budget.
- **Value:** at least the observation budget of the wait that ends in the
  marker's removal (for example `startup_timeout` for the creating-sentence
  wait). The marker is per stage, so a test holding two stages writes each
  stage's own budget (`tests/cli.rs` holds `ExtractingEmbeddedRuntime` and
  `CreatingDatabase`).
- **Ordering:** write the marker, with its content, before spawning the owner's
  process, as every caller already does, so the owner never reads a partly
  written file.
- **Why the same figure outlasts the test's wait** (inference from the call
  order, not a new mechanism): the owner's deadline starts when it reaches the
  stage, and the test's wait begins before the owner can reach the stage: the
  owner is a further process the spawned client starts, and it passes its
  earlier stages first. In `tests/cli.rs` the test's second wait begins as it
  removes the first marker, before the released owner can reach the second
  stage. So `stage_start + budget > wait_start + budget`: while the test is
  still waiting, the hold is still holding. When the test's wait itself
  expires the test has already failed, so the hold's later expiry changes no
  outcome.
- **Callers to adopt it in #212, not here:** `apps/kuru-tui/tests/terminal.rs`
  (`smoke` at ~2562, and `real_pty_first_launch_shows_the_creating_sentence_while_the_template_builds`
  at ~2855) and `apps/kuru-tui/tests/cli.rs` (~3522-3535, two markers; the budget
  is that test's own wait, today the flat `Streaming::WAIT`, which #212's
  derivation owns). Until they adopt it they behave exactly as today.

### Not changed

- `memory.startup_timeout_secs`, `hold_limit`'s default derivation, any product
  path, and any production environment reading. Everything stays inside the
  `cfg(any(test, feature = "test-support"))` seam.
- The marker's removal stays the release event; the 10 ms poll of the marker is
  its existing cadence, not a decision timer, and no retry is added.
- The in-crate users of the marker, activity.rs ~2905 and ~2974 (`std::fs::write(.., b"")`),
  write empty markers, so their meaning and code are unchanged and they keep
  exercising the default bound.
- The `cfg(test)` in-process `OpenHold` barrier and `OwnerHooks::hold`.
- No new helper, constant or environment variable; Windows owner forwarding
  (`forwarded_test_hooks`) forwards the same variable and reads the budget from
  the marker file, so it needs no change.
- `apps/kuru-tui` and `packages/kuru-memory/src/test_support*` are not edited.
- The slow render itself is not explained or fixed; the derived hold makes it
  harmless.

## Impact

- Files: `packages/kuru-memory/src/service/activity.rs` (the seam, its doc
  comment and its test module), the hook paragraph of `docs/development.md`
  (outside the brief's owned-file list; AGENTS.md requires workflow
  documentation in the same change, and the paragraph is far from the one hunk
  #208 edits at ~110), and this change's openspec directory.
- Coverage and CI: one test of virtual time and a temp directory; no engine,
  no subprocess, no added CI job. No product behavior changes.
- Coordination (measured with `gh pr view` on 2026-10-04, not blockers): #210
  (`test/memory-derived-waits`) and #208 (`fix/instrumented-child-outlives-test`)
  touch no region of `service/activity.rs`. #208 edits `service.rs`,
  `server.rs`, `test_support.rs` and `docs/development.md` near line 110; #210
  edits other kuru-memory files including `service.rs` and `test_budgets.rs`.
  #212 (`test/tui-derived-waits`) changes no `activity.rs` code and adopts the
  contract above only after this change merges.
- Affected surfaces: none of the four product surfaces (interactive, deploy,
  integration, agent-behavior); this is a test-support seam.

## Surfaces

- [ ] interactive
- [ ] deploy
- [ ] integration
- [ ] agent-behavior
