# Proposal

## Why

Native macOS CI37853387922 observed two membership attempts where a test incorrectly required one total attempt. The owner permits retry after an unobserved listing; successful readiness and a fresh observation after it is consumed remain required.

## What Changes

- Correct the owner-local attempt assertions in `packages/kuru-platform/src/unix.rs` to account for observed native `WouldBlock` refusals, retain exact pending/fresh-job checks, and finish owned cleanup before assertions.

## Impact

One existing native test changes; production behavior, deadlines, polling cadence and coverage gates remain unchanged. Existing held-spawn-lock coverage proves the actual refusal path.
