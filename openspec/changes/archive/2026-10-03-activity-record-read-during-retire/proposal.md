# Proposal

## Why

The publisher of an owner's open activity record writes on its own task, and since the progress count arrived (#174) the open always leaves one progress-only write for after its last stage: the store-ready milestones advance the count inside the 250 ms spacing, and the open's end then makes that write immediate. Nothing ordered that last write before the owner's close; the close joined the publisher only when it retired the record, after it had already retired the endpoint. So the record could still be replaced after the endpoint was gone. PR #198's Windows ARM run (37139918511, partition 8) failed `the_record_is_retired_after_the_endpoint_and_before_the_store_closes` with "the record was retired before the endpoint": the test waited only for the publisher's stages (`settled`), not its end, and its strict read at the endpoint-retired pause failed. The test discarded the read error, so the log could not show why. The retirement order itself was right on every OS.

In the product the client's reader was already safe. Only a missing name reads as absent, and a record replaced under a reader is unusable for that one poll. Even so, a starter whose attach fails once the endpoint is gone could read a late replacement and count it as progress, because nothing settled the record at that point.

## What Changes

- The owner's close awaits the publisher's last write after it stops accepting and before it retires its endpoint. From then on the record changes only by its failing mark or its retirement. The publisher joins once and keeps its last activity, so the failing mark of an owner that ends before its starter attached still writes from that activity.
- A deterministic regression test gates every record write until the listener has dropped. It requires that the close not reach the endpoint-retired pause while that write is pending, and that the record then holds the open's last activity. Without the fix it fails on every run.
- Every activity test that reduced a record read or lookup to a boolean now reports the error. `read_stages(...).is_ok()` becomes a read with context, and `Path::exists()`, which treats a failed lookup as absence, becomes `try_exists()?` through one helper. The flaked test waits for the publisher's end and asserts that the record is unchanged once the endpoint retires.

## Capabilities

### New Capabilities

### Modified Capabilities
- `project-memory-owner`: the record is settled before the endpoint retires (added requirement).

## Impact

- `packages/kuru-memory/src/service.rs`: `close_paused` awaits `Publisher::finish_writes` between the listener drop and the endpoint retire.
- `packages/kuru-memory/src/service/activity.rs`: `Publisher` joins once (`finish_writes`, cached `last`). `mark_failing` and `retire` use it. Tests changed as above.
- `docs/development.md`: the close order of the record.
- No change to the record format, the client reader, the platform replace, or any timing bound.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
