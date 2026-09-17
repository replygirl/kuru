## Why

Windows native CI failed a cold offline provision with `Dolt engine publication succeeded, but private stage cleanup failed -> Rejected removal at ...\.install-xchPUG\private: Access is denied. (os error 5)`. The private stage still holds the executed cold-probe copy of `dolt.exe`, and Windows refuses a checked delete of a just-executed image with `ERROR_ACCESS_DENIED` until its image section is torn down; the bounded cleanup window treats rejected removal as recoverable only for raw OS32, so it returned that first result without ever entering its existing two-second reconciliation. The same code passed this test on the neighbouring branch head while failing a different fixture there, so the holder is transient rather than a retained Kuru handle: `finish_published` already drops the candidate source and the checked probe before closing the stage.

## What Changes

- Treat a rejected private-child removal that reports native access denied as recoverable inside the existing two-second cleanup window, alongside the current rejected OS32 case and every uncertain result.
- Give the Windows cache-invalidation fixture the same symmetry, so its own bounded retry covers a denied removal of the binary its warm probes just executed in either reported phase.
- Keep the bound, the retained cache lease, the exact outer/child identity checks, the no-mutation pending waits and the first-cause error on exhaustion unchanged; a post-publication cleanup failure stays a reported error.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

`packages/kuru-memory/src/files.rs` Windows private-stage cleanup recovery predicate and its native-only fixtures, plus the `provision/native_tests.rs` Windows cache-invalidation fixture. No platform API, public setting, deletion policy, publication retry, process lifecycle or timeout change.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
