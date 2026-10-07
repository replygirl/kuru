# Proposal

## Why

The read-only inspection fixture compared full session JSON despite the documented
attached-versus-standalone presence distinction. macOS CI differed only in the
live_presence_known boolean; durable catalog fields and null driver were equal.

## What Changes

Parse both existing listings in apps/kuru-tui/tests/cli.rs, require null driver
and boolean presence in every row, remove only live_presence_known, then compare
every remaining field exactly. Preserve no-provision/status/history/revision and
limit assertions; production is unchanged.

## Impact

One existing fixture only; no new helper/API/test variant, deadline, guard,
provider or storage change. Acceptance is this exact case and existing app
closing guard, relevant statics and fresh full CI before merge/main.
