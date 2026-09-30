# Design

## Context

Since the owner became its own process the owner opens silently (`MemoryStore::open`, stdio to null), so the client sees no open stages from it. `MemoryStore::open_observed` reports stages through a `ProgressReporter` whose bitmask sends each stage at most once per reporter; the channel capacity is 16. The CLI renders stages as `Memory: …` labels (`memory_open_label`, `MemoryProgressOutput` in `cli.rs`). Unit 1 (merged) already passes a per-spawn starter token as the optional tenth service argument, the owner holds it in `OpenOptions::starter_token` for the whole open, and `ServiceOwner::close_paused` orders: drop listener, `record.retire`, `store.close`, `lock.release`. Because the owner retires immediately, back-to-back commands can meet a closing owner.

Sources of detail: `tmp/roadmap/unit2-open-activity-feedback-design-2026-09-29-revised.md` (the design), `tmp/roadmap/unit2-reanchor-2026-09-30.md` (re-anchor against `cb1c8e1d`, overrides the design) and `tmp/roadmap/unit2-lead-and-harness-decisions-2026-09-29.md` (overrides both).

## Goals / Non-Goals

**Goals:**
- One plain true sentence while memory opens, erased at ready; the owner publishes what it is doing.
- Feedback can never fail or slow an open; the record grants no authority.
- Opt-in marker lines so the open-time harness can measure without the `ready` line.

**Non-Goals:**
- No raised deadline, no retry, no sleep in tests (event hooks only), no weakening of isolation, ownership, recovery or the uncertain-write fence, no change to the meaning of `startup_timeout_secs`.
- No change to unit 1's argument position, parse rule or admission; no second token.
- No in-interface loading frame and no sentence for the period after ready (recorded follow-ons).
- No inference of activity from directories on disk.

## Decisions

### D1 Mechanism: the owner publishes an ordered stage list; the client forwards new entries

The owner records the cumulative ordered list of stages its observed open has begun, not only the latest. A missed poll or late write loses nothing. The client forwards entries it has not yet forwarded into the reporter it already uses, in order. Rejected: inferring activity from staging or install directories (layout about to change, and probing can fail a Windows checked move); a single current-value record (loses stages between polls).

### D2 Sentences (one constant each, in `apps/kuru-tui/src/memory_activity.rs`; last character U+2026, ASCII apostrophes)

| # | Constant | Text | Shown while |
|---|---|---|---|
| S1 | `OPENING` | `Opening this project's memory…` | Every open from its first moment, and whenever no more specific activity runs |
| S2 | `GETTING_READY` | `Getting Kuru's memory ready on this computer…` | This version's engine is not installed and the open waits for, unpacks or checks it |
| S3 | `CREATING` | `Creating this project's memory…` | The project has no active memory and Kuru creates it, imports older memory or finishes an interrupted setup |
| S4 | `UPGRADING` | `Upgrading this project's memory…` | An existing active store is upgraded to this version's format |
| S5 | `WAITING` | `Waiting for another copy of Kuru that is using this project's memory…` | Another process holds the owner lock or the owner's startup lock and the open cannot go on |

Leftover-file notices N1, N2 and N2' (the latter when `memory.cache_dir` is set) are kept as reworded constants in the same module and written on their own line after a successful in-process open. Rejected: a `Memory:` prefix or ready line (removed by decision).

### D3 Stage mapping (`next_sentence(current, stage) -> Option<Sentence>`, `None` keeps)

| Rule | Stage | Result |
|---|---|---|
| R1 | `WaitingForProjectOwnership` | S5 |
| R2 | `WaitingForRuntimeCache`, `ExtractingEmbeddedRuntime` | S2 |
| R3 | `VerifyingRuntimeCache`, `PreparingDatabase`, `StartingMemoryService` | S1 |
| R4 | `CheckingRuntimeVersion` | keep if current is S2, else S1 |
| R5 | `CreatingDatabase` | S3 |
| R6 | `UpgradingDatabase` | keep if current is S3, else S4 |
| R7 | `OpeningDatabase` | keep |
| R8 | `Ready` | clear; retained-install stages are held back for N1/N2 |
| R9 | any other or future stage | keep |

A specific sentence never appears before its work begins; it can stay up to one 100 ms poll (plus the owner's endpoint tail) after the work ends. With a fresh store initialised and migrated inside the stage worker, R6's keep case now fires only for a recovered older stage that upgrades on the active path. Known limit: the client and the forwarded owner stages share one reporter bitmask, so a repeated wait after a client wait shows S1, which is true but less specific.

### D4 Stage sites

- `CreatingDatabase`: first statement after the read-only `ensure!` in the `!Self::exists` branch of `open_inner`.
- `UpgradingDatabase`: first statement inside the writable `found < CURRENT_VERSION` block, above the `#[cfg(test)]` pair that calls `run_migration_worker`.
- `WaitingForProjectOwnership` in `open_inner`: the unconditional report is deleted; `acquire_lock_reporting(file, duration, progress)` next to `acquire_lock` reports once on the first `WouldBlock` before the first sleep, same deadline, 25 ms retry and error text; `acquire_lock` becomes a wrapper with a silent reporter.
- Client, in `attach_or_spawn_elected`: `WaitingForProjectOwnership` between the owner-probe `if let` block and its `ensure!`; `StartingMemoryService` right after `let probed`; in `attach_existing`, `WaitingForProjectOwnership` inside the Start-lock block right after the owner-free early return. `attach_or_start` and `attach_existing` keep their signatures and pass a silent reporter to the new `_observed` variants; the six silent reattach callers are unchanged.
- The facade's unconditional waiting report is deleted; the read-only fallback uses `activity::open_local_forwarding`, which runs `open_observed` in process and forwards every stage, including ready and the retained-install stages.
- `progress.rs` adds the three variants and bits, `ProgressReporter::is_observed()` and a const assertion that the variant count is at most the capacity of 16.

### D5 Record: `<data>/memory/services/<hash>/activity.json`

```json
{"format":1,"tag":"<lowercase hex sha256>","stages":["CreatingDatabase"]}
```

`tag = hex(SHA-256(b"kuru-open-activity-v1\0" || token.as_bytes()))` using the `sha2` already used in `service.rs`. `serde(deny_unknown_fields)`; stage names from a private explicit match (`MemoryOpenStage` is `#[non_exhaustive]`, no serde); an unknown name makes the record unreadable for that poll. Excludes `Ready` and the retained-install stages. Written by `crate::files::write` (atomic, `ReplaceRegular`) in the private directory; read by `crate::files::read_bytes` with a 4 KiB limit. No authority: election, attachment, recovery and retirement never read it. The client computes the same tag from its local token, only when `progress.is_observed()`. Rejected: writing the raw token (any same-user reader could present it and mark the owner reached early; unit 1 keeps it out of `Debug`); a second token or an "observed" argument (extra protocol surface for no benefit); a version-4 check (would change unit 1's parse rule).

### D6 Publisher and client read

`activity::open_owner_store(options)` reads `options.starter_token`. With none, and outside test support, it calls `MemoryStore::open` as today and publishes nothing (untokened owner). Otherwise it runs `open_observed`, selecting biased toward the stage receiver, feeding a `tokio::sync::watch` to a publisher task whose writes run in `spawn_blocking(files::write)`; after a failed write it waits 100 ms or a newer list and writes the latest. Nothing in the open, endpoint publication or `serve` awaits the task. Under `test-support` it absorbs `open_owner_store_with_fixture_stages` and applies two hooks forwarded to the owner on Windows next to `STARTUP_STAGE_DIAGNOSTIC_ENV`: `KURU_TEST_MEMORY_OPEN_HOLD_DIR` (hold the open after stage `X` until `<dir>/<X>.hold` is removed, once the publisher reported a list containing `X`) and `KURU_TEST_MEMORY_ACTIVITY_WRITE_FAILURE=1`. `fixture_startup_observations`' explicit name list gains the three new variants.

Client: `activity::forward_new(data_dir, scope, &tag, &mut forwarded, progress)` sits after the deadline block and before the 100 ms sleep in the post-spawn loop. The deadline and `polls` are untouched, no read follows a successful attach, and a missing record, IO error, decode failure, wrong tag or identity-mismatch is ignored for that poll, never logged and never disabling reads.

### D7 Retirement (replaces the design's detached-task removal)

The publisher task stops without removing anything when the open future completes; its handle is kept as `ServiceOwner.activity: Option<activity::Publisher>`. `close_paused` calls `activity::retire(publisher, &data_dir, &scope)` immediately after `record.retire` and its `AfterEndpointRetire` pause and before `store.close()`: join the finished task, then rename to `activity.retired` and remove, only for a record with this owner's tag, errors ignored. `ServiceOwner::open` retires on both error returns: `open_owner_store` joins and retires before returning `Err` on a store-open failure, and the `prepared` error branch calls `retire` before `store.close()`. This runs under the owner lock, so no successor can exist and a detached task can never remove a successor's record; rename-then-remove follows the endpoint's proven Windows pattern where a removed name stays occupied while a client holds it. Rejected: a detached remover (races the lock release, since retirement is immediate); never removing (safe because the tag filters it, but leaves a file per project).

### D8 Rendering (`memory_activity.rs`, moved out of `cli.rs`)

`open_memory` gains `interactive`, writes S1 at the start, drains queued stages with `now_or_never` and renders once. Terminal: `\r` + text + padding to the widest line written, widths by `unicode-width` `width_cjk`; sentences shortened to `cols - 1` keeping the ellipsis, nothing written when there is no room; erase on ready and on failure. Not a terminal: each new sentence once on its own line, never repeated, nothing at ready. Sentences go to stderr, or to stdout only for an interactive session whose stderr is not a terminal, before the interface takes the screen. A failing sink switches output off. `memory_open_label` and its tests are deleted; `cli.rs` call sites pass `cli.command.is_none()`.

### D9 Markers

Enabled iff `KURU_OPEN_MARKERS` is exactly `1`, read once per `open_memory`; release code, not `test-support`. Line: `kuru-open-marker v1 <event> <ns>\n`, ASCII, one `write_all` plus `flush`, always to stderr; `<ns>` is `ANCHOR.elapsed().as_nanos()` with a `LazyLock<Instant>` forced at the top of `open_memory`. `open-start` is written after `open_managed_observed` returns and before the first poll of the open (so before any attach attempt, and before S1); `waiting-ownership` once, when R1 first fires (conditional reports only, so not on every open); `ready` at `complete()` on the `Ok` arm after the erase, including an attach to a running owner, not on `abandon()`. When the sentence sink is the same terminal as stderr, a marker is written as erase, marker line, then redraw of the current sentence; for `ready` the erase already happened. A marker write error is ignored. Documented in `docs/development.md` only.

### D10 Version compatibility

A pre-unit-1 client spawns no token, so no record and old lines. The current main client against a unit 2 owner: the owner publishes a record nobody reads and retires it at close. A unit 2 client against a current main owner: no record, S1, and S5 when the client waits itself. A unit 2 client against a pre-unit-1 binary fails as it already does since unit 1.

## Operational surface

- Bind address and network: none; the record is a private file in the owner-private service directory and no port or connection is added.
- Container versus runner: no change; CI runs the existing native test jobs, and the Windows legs verify the two forwarded test-support variables and T8.
- Secrets: none. The record carries a SHA-256 derived tag, never the starter token, and the token is not treated as a secret by unit 1.
- Connection limits: none added; the client adds one synchronous read of at most 4 KiB per poll, after the deadline check.
- Binary versions and arches: the marker lines are release-binary code on every target; the test hooks exist only under `test-support`. Older and newer binaries interoperate as in D10.
- Configuration: `KURU_OPEN_MARKERS=1` is the only new variable in the release binary, documented in `docs/development.md` only.

## Risks / Trade-offs

- [The waiting sentence now appears after any quick re-run, since the previous owner may still be closing] → The sentence is true (that process is Kuru and still uses the memory); report the changed frequency to the lead; tests that expect only S1 await the previous owner's exit first.
- [A persistent fault on `activity.json` alone can leave the last forwarded sentence showing] → The open is unaffected; the same fault would break `EndpointRecord::publish`, which fails the open and erases the line.
- [Record cost on every owner start (about ten small atomic writes on the blocking pool)] → Never awaited; release-smoke medians must stay within noise of 6173 ms first launch and 628 ms cold-existing, awaiting the previous owner's exit between iterations.
- [Windows replace or remove while a client holds the record] → Rename-then-remove and the retry absorb it; proven only by native Windows CI (T8), unverified on macOS.
- [Sentence lag up to one poll plus the endpoint tail] → Bounded and stated in the spec; never before the work begins.
- [`open-start` precedes S1, so the harness's first-stderr-line fallback sees the marker when markers are on] → Accepted; with the variable unset S1 is the first stderr line.
- [Scripts searching stderr for `Memory:` stop matching] → No repository workflow or script does; the harness moves to markers.
- [Conflict with the template-copy change in `open_inner`] → Stage sites are described by logic and kept last (WP-E) so they can be re-applied.
