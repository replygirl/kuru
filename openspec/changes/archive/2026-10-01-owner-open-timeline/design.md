# Design

## Context

Source design: `tmp/roadmap/unit6b-owner-timeline-design-2026-10-01.md` (read at `origin/main` 1c93476f, whose section numbers are cited below). The measured split is `tmp/roadmap/unit6-owner-start-split-macos-2026-10-01.md`. Requirements are in `specs/project-memory-owner/spec.md`; this document records how.

The memory owner is its own process. Its open runs between the supervisor's Ready and the service endpoint publish, then it serves, and `close_paused` orders: drop listener, retire endpoint, retire activity record, close store and reap Dolt, release the owner lock. A successor's wait ends at that release. On Unix the owner inherits the client's environment; the supervisor is spawned with `env_clear` and PATH only. On Windows the owner is spawned with an explicit minimal environment, so the variable never reaches it.

An earlier unpushed attempt (`cf05375e`) was discarded rather than patched: its tests shared the process static, one test was empty, it wrote through a symlink-following, umask-governed, truncating open, and its proposal carried off-arm medians as on-arm results. None of it carries over.

## Goals / Non-Goals

**Goals:**
- Split the 249 ms stage into named rows on one monotonic clock, in the release binary, only when asked.
- An aged-store fixture through the real write paths, usable from a script against a release binary.
- Zero effect on the open, serve, close, a predecessor's close or any successor's lock wait, on or off.

**Non-Goals:**
- Implementing any optimization candidate (R1-R3). A row that grows becomes its own unit, designed with the lead first.
- A supervisor frame, client marker or progress stage change.
- Windows support, Ubuntu numbers (assistant3's harness), durable or readable-by-product records, retention or cleanup of timeline files.
- Any change to the harness branch.

## Decisions

### D1 Module `open_timeline.rs`, crate-private, installed only in `service_entry`

A `OnceLock<Timeline>` is set by `install_from_env()` as the first statement of `service_entry`, which both binaries reach and no test calls in process, so test runners never install it. `enabled` is a pure function true only for exactly `1`. Each stamp site calls `stamp(Event)`, which is one `OnceLock::get` and an untaken branch when off. The module sits at the crate root because its sites span `server`, `store`, `usage_ledger`, `provision` and `service`; placing it under `service/` would make lower layers import upward. Rejected: reading the variable at each site (an env read per site), and a `cfg` for Windows (the module compiles and lints everywhere; nothing is claimed there).

### D2 Eighteen events, one site each, bounded and sealed

Events: `owner-main` (the anchor), `owner-lock`, `cache-verify-start`, `cache-verify-end`, `supervisor-ready`, `probe-verified`, `main-pool`, `version-read`, `validate-active`, `candidate-recovery`, `usage-pool`, `usage-scan-1`, `usage-upgrade`, `usage-validate`, `usage-scan-2`, `store-ready`, `listener-bound`, `endpoint-published`. The stage under study is `endpoint-published - supervisor-ready`, its 13 consecutive differences are the rows; the first four split the time before it. `Timeline::stamp` reads the clock while holding the log mutex, so stored order is non-decreasing across threads. Capacity is 64, allocated once; an overflow counts `dropped`, never reallocates or panics. `endpoint-published` seals the log and later stamps only count `late`, so the record describes the open alone. `validate_branch` returns the rows it already decodes and the first scan's count is recorded as `usage_rows` (name and number only). Rejected: a growable log (allocation under the stamp), and unsealed logging (repeats from later paths pollute the open).

### D3 One write, after the lock release, plain and non-durable, via the checked handle

`close_paused` writes after `lock.release()`, whatever `retired`, `closed` and `released` hold, and never changes the returned result. Writing earlier would lengthen exactly the wait the release ends. The write opens the existing services directory with `files::open_directory` (no directory creation after authority is lost) and uses `Directory::create_new` for `open-timeline-<generation>.json`: owner-private, `0600`, `O_EXCL`, relative to the held directory, no sync, no rename. `files::write` is rejected because it stages in a subdirectory and publishes with fsync (`F_FULLFSYNC` on Apple targets, an fsync class of cost at every gated exit). A crash mid-write can leave a truncated file; readers treat unparsable as missing. A per-generation name means a predecessor's write cannot collide with a successor's.

### D4 Schema `kuru.open-timeline` v1, serialized with `serde_json`

Fields: `format`, `format_version`, `kuru_version`, `service_generation`, `anchor_unix_ns` (`u64`, the only wall-clock value), `events` (name, `ns`), `counts.usage_rows` (`null` if never reached), `dropped`, `late`; at most 8 KiB. Every string is a static name, the package version or the generation UUID. Tests search the bytes for the data directory, project path, scope, scope hash, connection secret and store instance.

### D5 Tests use local `Timeline` values; only the child owner touches the static

Unit tests build local timelines (a test-only `with_capacity`). A `#[cfg(test)]` `timeline` knob on `ServeKnobs` lets in-process owner tests hand `close_paused` a timeline without installing the static. Child-owner tests (Unix) run a real owner with the variable through the existing owner-environment scope (made `pub(crate)` outside `mod tests`) and a `FixtureLoggedOwner::exited` that awaits the process exit under `fixture_deadline`, with no polling or sleep. Existing stage-sequence tests, unmodified, are the evidence that the unset path is unchanged.

### D6 Aged-store fixture in `kuru-memory` test-support, run through the package binary and a mise task

`test_support/aged_store.rs` is compiled only under `test`/`test-support`; the `age-store` arm in `main.rs` follows the existing `prefetch` precedent, and `measure:age-store` sits beside `measure:lifecycle`. It locates the single existing scope, waits for the previous owner's release, holds the owner lock for the run, opens the store directly with the prepared supervisor snapshot and `offline = true`, then per conversation follows the runtime's write order (create session and mark new; per turn: admit checkpoint with the turn journal, the possible-dispatch journal put, user append, usage admit, observe, settle, assistant append, settle checkpoint with the completed journal) with namespaces from the same `kuru-core` memory policy. Content derives from a self-contained SplitMix64 over `(seed, index)`, so logical content (ids, transcripts, ledger rows) is deterministic; Dolt commit hashes are not. Volume per conversation is `2+8T` writes and `1+3T` usage rows. The source design's `2+7T` omitted the runtime's possible-dispatch journal `put` (`kuru-runtime` `mark_possible_dispatch`); the fixture copies it, so formula and test 10 were fixed together, never padded. Rejected: a new `[[bin]]` (a target for one measurement), an `#[ignore]` test (no size/seed arguments, fixture roots deleted at teardown) and a `kuru-delivery` tool (must not compile memory).

### D7 Size ladder chosen by a pilot, not assumed

No per-write cost record exists. A 100-conversation pilot in three parallel processes gives writes/s `r`; the top size is the largest of 20k/10k/5k with `9*C / r <= 2700 s`, the ladder is `{C/20, C/4, C}`, and a run whose rate collapses is discarded, not reported at its nominal size. Dolt auto-GC (`SERVER_BEHAVIOR`) is recorded as a confounder via `du` and one untimed warm-up open per store.

### D8 Measurement is outside the archive gate

The macOS split is recorded in the roadmap notes with its evidence; this change's verification cites it but does not wait on Ubuntu.

## Risks / Trade-offs

- [A stamp site costs the unset open something] -> one acquire load and an untaken branch; the A/B observer row (Dolt endpoint to service endpoint, instrument on vs off) is reported against its +/-2 ms bracket, and any gap is reported as the instrument's cost.
- [Timeline files accumulate in a developer's data directory] -> documented; the measurement script uses scratch directories and removes them; no product code reads or deletes them.
- [A write after lock release delays owner process exit] -> by one small write, which no client `ready` or lock waiter observes; tested for ordering and failure.
- [Aging takes hours at the top size] -> the pilot-derived ladder and a discard rule bound it.
- [The usage-row formula differs from the runtime sequence] -> reconciled in the fixture task and test 10 together.
- [Windows-only code drifts] -> the module has no `cfg`, `lint` covers the Windows target, and `FixtureLoggedOwner::exited` is `cfg(unix)`.
