## 1. Facade fixture deadlines

- [x] 1.1 Add a facade test deadline derived from `provision::LOCK_TIMEOUT`, `startup_timeout_secs` and `QUERY_TIMEOUT`, scaled by real service lifecycles, and verify by review that each of the 17 outer fixture timeouts in `packages/kuru-memory/src/facade.rs` uses it with the lifecycle count read from its body, while inner short waits and assertions are unchanged.
- [x] 1.2 Run the facade tests through `mise run //packages/kuru-memory:test` (with prefetch, `--all-features`), repeat `candidate_begin_recovery_keeps_clones_fenced_until_exact_ref_reattaches` several times, and record the observed results.
- [x] 1.3 Reproduce the cold first-batch shape locally (empty `KURU_DOLT_CACHE`, first-batch fixtures concurrent, with and without a throttled load simulation) and record timings, naming any check that could not be run.

## Lifecycle counts

`fixture_deadline(n)` = `LOCK_TIMEOUT` (180s) + n × (2 × `startup_timeout_secs` + `QUERY_TIMEOUT`) = 180s + n × 90s with the default budgets.

- n = 1 (270s): session lifecycle, fork, mode checkpoint (was 60s), public turn, accepted cancelled write, remote reasoning summary, remote session checkpoint, candidate promotion, candidate conflict, usage recovery, dream lease, managed facade (its competing local open is refused under its own inner 5s bound).
- n = 2 (360s): public transcript and legacy continuation (local seed open, then owner).
- n = 3 (450s): candidate begin and candidate unit (one owner, then an owner and its restarted successor).
- n = 6 (720s): selected abandonment (two iterations of local open, owner and successor; was 150s).

Inner waits (pause notifications, witness polls, cancellation, 5–20s permit/reap waits, fast refusals) and all assertions are unchanged.

## Observed evidence

Failure analysis of Windows coverage run 36210125264: the libtest timestamps show 2 test threads. `catalog…` finished in 0.05s, then `cancelling…` and `candidate_begin…` both started against a cold `%TEMP%\kuru-dolt-test-cache`. The shard ledger shows `kuru_memory`'s lib binary runs first and the shard never runs `prefetch`.

| Job | `cancelling…` (cold start) | `candidate_promotion…` (after warm) | `candidate_begin…` (cold start) |
| --- | --- | --- | --- |
| 108314702541 | 9.2s | 6.3s | 17.2s |
| 108314707937 | 15.7s | 9.1s | 27.7s |
| 108314713444 | 14.0s | 9.3s | 25.7s |
| 108314719071 (failed) | 77.0s | 10.7s | >90s (timeout) |

Only the cold-start pair shifted, by about 60–70s. The logs cannot show which step inside the cold install was slow on that runner.

Local macOS aarch64 run, 2026-09-25, on a shared machine (load average 18–38 on 14 cores from a concurrent coverage run):

- `mise run //packages/kuru-memory:test -- --lib facade::tests` (prefetch, `--all-features`, prepared supervisor): 17 passed, 0 failed (76.96s).
- `candidate_begin_recovery_keeps_clones_fenced_until_exact_ref_reattaches`, 8 repeated runs on a warm cache: all passed, taking 19.2, 20.6, 36.8, 37.4, 41.9, 23.7, 21.2 and 24.3s.
- Cold `kuru-memory prefetch` into an empty cache (uninstrumented macOS tar.gz): 3.8s.
- Cold first batch before the change, 4 threads (all three started at once), empty `KURU_DOLT_CACHE`: `cancelling…` and `candidate_promotion…` took 9.2–10.0s and `candidate_begin…` 14.6–16.8s, compared with 4.3s, 4.0s and 10.2s when warm and run alone. That is the same uniform shift. Under `taskpolicy -b`, the batch took 31.0, 31.3 and 47.5s.
- Cold first batch after the change, 2 threads (the CI thread count), empty `KURU_DOLT_CACHE`: without throttling, `cancelling…` took 23.0s, `candidate_promotion…` 31.7s and `candidate_begin…` 34.3s. Under `taskpolicy -b` as a load simulation, both cold fixtures passed libtest's 60s warning; `cancelling…` took 68.8s, `candidate_promotion…` finished at 81.4s (12.7s after the cache was warm), and `candidate_begin…` took 83.2s. This is 6.8s under the old flat 90s bound and within its derived 450s bound. All passed.
- `mise run //packages/kuru-memory:lint` (clippy `-D warnings`) passed, and `cargo fmt --check` is clean.

Not run: Windows native coverage on the exact head. It is a before-merge CI gate. A local Windows or instrumented run was not available on this host.
