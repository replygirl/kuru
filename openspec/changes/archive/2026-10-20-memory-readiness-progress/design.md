# Design

## Context

The elected starter's readiness wait is a no-progress window: `memory.startup_timeout_secs` restarts whenever the owner's tagged activity record changes (`service.rs` readiness loop), and the record changes when a stage is reported or the open's `OpenTicks` count advances. `progress::milestone` pairs a timeline stamp with an advance, but several stamps inside a creation were bare `open_timeline::stamp` calls. On the template path the owner reports `OpeningDatabase` (`store.rs`) and then hands the startup lock to the creation worker, whose copy, identity write and stage engine start advanced nothing until the engine's `supervisor-ready`: `supervisor-spawned` and `channel-accepted` (`server.rs`) were bare stamps, the copy had no ticks, the stage worker reported through `ProgressReporter::silent()`, and the opener's `template-copied` and `activated` were bare too. In CI run 39455388792 (job 118741241523) a degraded windows-latest runner stretched `create-start → supervisor-spawned` to 28.6 s; the starter's last observed change was the `OpeningDatabase` write about 5.8 s after spawn, so at 30 s without change it abandoned an owner that was still working (`since-progress=30009ms` against `since-last=7024ms` for the owner's own last stamp).

## Goals / Non-Goals

**Goals:**
- Every completed step of the creation span between the `OpeningDatabase` report and the stage engine's `supervisor-ready` advances the open's own count, on the template and cold paths alike, without changing stages.
- The timeline names which step of `create-start → supervisor-spawned` was slow.
- Stall detection is unchanged: an owner that truly stops still fails one window after its last advance.

**Non-Goals:**
- No change to `memory.startup_timeout_secs`, `SUPERVISOR_TRANSPORT_ALLOWANCE` or any timeout value; no rerun-based cure.
- The engine's own `startup_deadline` (`server.rs`, computed before the supervisor spawn) is not moved; see follow-ons.
- No Cargo, TUI, record-format, stage or protocol change.

## Decisions

- **Milestones at the existing sites.** `SupervisorSpawned` (Unix and Windows), `ChannelAccepted`, `TemplateCopied` and `Activated` call `progress::milestone`, exactly as `SupervisorReady` already does. Each is a distinct one-shot point or a completed bounded unit, which is what the `OpenTicks` contract admits; none is on a timer, a retry iteration or a lock wait. Rejected: a periodic owner heartbeat, which would let a hung owner hold its starter forever and violates the contract.
- **Template copy and identity write.** `copy_blocking`, the single site every template copy passes through, takes the open's ticks and stamps `copy-end` as a milestone once a copy succeeds (one stamp site; a build-then-copy open reaches it once). The creation worker advances the count once its stage identity record is written; no timeline event is added for that step. Rejected: `ByteTicks` inside `copy_hashing` — a template is a few hundred KiB, far below the 8 MiB unit, so it would never advance.
- **Counting stage worker reporter.** `ProgressReporter::counting(ticks)` has no stage sender, so the creation worker's stage reports stay invisible to the client (the opener's `OpeningDatabase` report still stands for them, rule R7), while each report and milestone advances the open's count. Rejected: forwarding the worker's stages, which would change the client's stage sequence.
- **Localizing events.** `legacy-prepared` (after the legacy import probe or its suppression), `recovery-checked` (after `recover_staging`), `copy-end` and `spawn-call` (immediately before each supervisor spawn call; on Windows after the private listener bind, on Unix after the test-only child-creation gate) are new canonical events, each a progress milestone. `spawn-call → supervisor-spawned` is then the process creation itself.
- **Hold order unchanged.** `milestone` still stamps before it advances, so a test hold at an event sees every earlier advance but not that event's own. The regression test therefore discriminates on the advances before the first `supervisor-spawned`.
- **Bare stamps left.** `create-start` (directly after the `CreatingDatabase` report), `extract-*` (inside byte-ticked extraction), `migrate-*` (around per-step advances) and `cold-created` (after the cold stage start's ticks) each sit beside work that already advances, so they are left as they are to keep this fix to the silent span.

## Risks / Trade-offs

- [More record writes] → count-only changes are already coalesced to one write per 250 ms; an open adds about six advances per creation and three per engine start.
- [A slow single engine start still fails] → by design: the engine's own deadline bounds each start, and the starter now fails with the owner's precise error or a stall named at the slow step rather than abandoning a working owner.
- [`channel-accepted` cannot be held in tests] → the fifo event hold is Unix-only and the event is Windows-only; the Windows site is covered by Windows-target lint and by native Windows CI exercising the same helper.

## Follow-ons

- Anchor the engine's `startup_deadline` (`server.rs`) at child creation, or split a pre-spawn bound, so time before `CreateProcessW` returns is not charged to engine readiness. It changes a product budget and needs its own decision.
- Consider making `create-start`, `cold-created`, `extract-*` and `migrate-*` milestones too, so the documented "every stamp is a progress point" holds without exceptions.
