# Proposal

## Why

`cancelled_dream_abandons_candidate_hook_annotations_and_reaps_hook_descendants`
failed on PR #197 run 37138704397 (macos-latest coverage partition 2, job
111248598206) with "hook descendant survived dream cancellation". The test
proved descendant reaping by timing: the hook backgrounded
`(sleep 1; printf survived > file)`, the test cancelled the dream, waited for
it to return, slept 1200 ms and asserted the file was absent. That assumes
cancellation through reap finishes within about one second of the marker; on
a slow, instrumented runner the reap legitimately finishes later, the
descendant writes first, and the test fails although nothing survived the reap.

The product path is not implicated. The hook root runs in its own owned process
group (`OwnedProcessGroup::spawn`, `process_group(0)`); a backgrounded subshell
of non-interactive `/bin/sh` stays in that group (measured); cleanup sends
SIGKILL to the group before the root, reaps the root, and returns only after
`presence_after_reap` reports the group absent; an unconfirmed cleanup surfaces
as an `Event::Error` from `await_hook_cleanup`, not in the dream's result, so
the test checks the group directly.

## What Changes

- The test's hook backgrounds `(publish $$; sleep 30; printf survived > file)`:
  the descendant itself publishes the owned group id (`$$`, the root pid that
  leads the group, written atomically through `tmp` + `mv`) to the existing
  marker path, so the marker wait fires only once a descendant is running. The
  descendant outlives the 5 s hook timeout and every bound in the test, so
  within the test only a reap can empty the group.
- After `dream_controlled` returns, the test reads the group id from the marker
  and requires `kuru_platform::unix::observe_group_after_reap` to report none
  of our processes in the group (signal zero plus one listing; it never
  signals). On failure it prints the surviving members, and whether the
  descendant outlived its 30 s sleep (the `survived` file, evidence only).
- The fixed 1200 ms sleep and the `!survived.exists()` assertion are removed.
- Only the hook script string and the test's tail change; the region between
  them is left byte-identical so PR #181, which rewrites that region, merges
  without conflict (measured; see blocking-changes.md).
- No pre-cancel positive control: it would sit in #181's region. The id is the
  group id by construction (`process_group(0)`), membership of a `( … ) &`
  subshell was measured, and an unclassifiable number (0, 1, other errno)
  reads `Unobserved`, which is not `none_of_ours`, so a garbage id fails
  rather than passes.
- No product code, documentation or dependency change.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

- `packages/kuru-runtime/src/hook_tests.rs` (this one test only).
- Test only; no runtime, connector or platform change.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
