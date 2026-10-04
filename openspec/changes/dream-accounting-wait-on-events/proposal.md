# Proposal

## Why

`accounting_tests::abandoned_dream_keeps_usage_after_reopen_without_advancing_main`
decides its outcome with a flat `tokio::time::timeout(5 s, observed.notified())`
(`packages/kuru-runtime/src/accounting_tests.rs`, around line 3600; its expiry
panics with "the dream provider call was not reached within 5 s"). It failed on
PR #213 run 37182321959 (attempt 1), windows-latest coverage partition 6, job
111377579850, panicking at `accounting_tests.rs:3673:5` after 43.62 s in
that test binary (32 passed, 1 failed). Measured from the 0add315b step
recorder in that log, every step advanced while the flat bound decided the
outcome: dream started +586.6 ms, candidate begun +1849.8 ms, private history
reads +2816 to +2835 ms, prompt inputs appended +3433.8, +3810.7 and
+4183.7 ms, the first actor's provider stream entered at +4564.0 ms, the others
at +5176.9 ms, report at +5594.7 ms. After cancellation the task finished
`Err(turn cancelled)`, so nothing was stuck; the 5 s literal is a number guessed
at runner speed. Earlier occurrence: PR #142 run 36778921639, job
110104109402 (same test, same panic site class; recorded in the flaky-test
catalogue). The PR #213 diff (kuru-tui managed-memory fixture and one
kuru-memory test-support helper) does not touch this path (inference from the
diff scope). Repository rule: no flat wait decides an outcome; a wait ends on
its event under a budget derived from a product value.

## What Changes

- In `packages/kuru-runtime/src/accounting_tests.rs`, replace the flat 5 s wait
  in `abandoned_dream_keeps_usage_after_reopen_without_advancing_main` with an
  event wait: the provider-call notification (`observed`) versus the dream
  task's completion, bounded by the fixture's existing derived `gap`
  (`progress_wait::unhooked_gap_bound`, which is
  `tests::turn_admission_deadline()`, the memory startup budget from which the
  Dolt listener derives its statement read timeout), re-armed by the same
  progress signals (`TaskWatch::progress`) the post-cancel join already uses
  (#181, #206).
- If the dream task finishes before the notification, the test reports the
  task's outcome (`Ok(report)` or the `Err` chain with the cancelled flag)
  instead of a timeout. If one whole gap elapses with no progress and neither
  event, it fails with the existing step-recorder report
  (`explain_expired_dream_wait`, adapted to name the gap rather than "5 s").
- Shared runtime test support in `packages/kuru-runtime/src/progress_wait.rs`
  gains the notification-versus-task wait (or the call site composes the
  existing `until_event`), with paused-clock regression tests next to the
  existing ones.
- Non-goals: other flat waits (another PR sweeps them), product code, any longer
  literal, retry, the step recorder's content.

## Impact

Test-only: `kuru-runtime` `cfg(test)` modules. No product, documentation or
workflow change. Test runtime is unchanged on a healthy run (the wait returns on
the notification); a stalled run now fails after one silent gap (30 s locally)
instead of 5 s, with the same diagnostics.
