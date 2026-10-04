# Design

## Context

The exact main native macOS failure reached the existing eight-second candidate pool-close deadline after dream cancellation. Its diagnostics contain no SQL-session or release-task trace, so this change does not claim the historical cause is proved.

Pinned SQLx 0.9.0 drops a normal PoolConnection by spawning return_to_pool. Its floating connection retains the pool's size/semaphore guard, runs after_release and then pings. MySQL ping first drains an unfinished query and queued rollback through wait_until_ready; this path has no return-task timeout. Pool close waits for that retained permit. If close marks the pool closed before the task starts, SQLx instead directly closes it, so that race must be controlled in the regression.

## Goals / Non-Goals

**Goals:** prove the canceled-query mechanism with real Dolt; make abnormal pooled-session disposal bounded without returning its permit before socket close; preserve successful release and exact retirement/reconciliation gates.

**Non-Goals:** raise runtime or fixture budgets, detach server sessions to relax capacity, alter runtime cancellation, add dependencies, redesign pooling, or infer a CI cause from elapsed time alone.

## Decisions

1. On PooledSession Drop, flag a retained SQLx connection close_on_drop before dropping it. SQLx keeps its floating permit while take_and_close runs under its existing five-second close timeout. MySQL close flushes buffered rollback and Quit, then shuts down its owned socket without waiting for pending results. The timeout is below the existing eight-second pool-close allowance, but cannot prove scheduler progress. Normal release takes the connection first and still returns it inline; canceled release already drops its floating connection without this Drop path.
2. Keep min_connections zero and all existing pool-close and exact server-session gates. Detach was rejected because it releases the permit before server cleanup and may overlap a replacement server session. The fixture's max-one assertion is about client pool permits, not server-side absence.
3. Use an opaque loopback relay with bounded eight-KiB buffers to the fixture's own real Dolt. Authenticate and begin before arming it, then hold the first server response to a raw SELECT 1. Reaching the response gate proves native execution while the client query remains pending; the relay never decodes or logs authentication or query bytes.
4. Run the causal tests explicitly on a current-thread Tokio runtime. A native SQLx after_release callback signals synchronously before the return task reaches the held-response ping. The test cannot resume until that task yields. For the corrected path, the relay observes client EOF instead. SQLx MySQL close has no await after socket shutdown; its Tokio socket delegates to TCP poll_shutdown, which returns Ready after shutdown(Write). The floating size guard drops in that same close-task poll. The relay cannot run between EOF-causing shutdown and guard release on this runtime, avoiding a multithreaded EOF/permit race.
5. After either causal event, poll pool.close once and require Ready while the query response is still held. The old started-return path is Pending. A separate pre-close control marks the pool closed before yielding and confirms the direct-close race. Collect the outcome, then unconditionally release the gate, close the pool, join the relay and close the store before returning the regression error. If the relay join times out, abort and await that owned task before propagating the failure. A returned join error already proves task completion; do not poll a completed JoinHandle twice. Drop is only a last-resort fallback, not evidence of a completed join.

## Integration contract

The fixture clones only its own store's SQLx connection options, redirects host/port to its local relay and never prints credentials or options. It uses the existing prepared supervisor and verified offline archive. Its raw fixture pool is max-one/min-zero and wrapped by MemoryPool::fixture; native SQLx after_release is the only return-task control. Tests observe exact connection IDs separately through the live owned store before declaring server retirement. Receipt-bearing WriteSession and resolve_uncertain are unchanged and remain covered by their existing real-Dolt controls.

Owned source is pool.rs and its new private pool/cancellation_tests.rs module. All other changes are this fix's typed artifacts and narrow lifecycle spec addition.

## Risks / Trade-offs

- Abnormal disposal discards a connection that could otherwise be reusable. Successful explicit release, commit and rollback keep inline reuse; test their IDs and pool counts.
- SQLx's close task is scheduled asynchronously and carries a five-second dependency bound. Keep the existing owner wait and exact server-session proof; do not claim the dependency timeout guarantees an eight-second scheduler deadline.
- An EOF can race permit release on multithreaded runtimes. The causal fixture uses current-thread scheduling and the pinned no-yield shutdown path; no arbitrary sleep is introduced.
- An expected negative assertion can leak a gated connection if raised early. Record the assertion as data and finish relay, pool and store cleanup on both outcomes before reporting it.
- Hosted Linux and Windows behavior cannot be inferred from local macOS evidence. Record fresh exact-head native CI separately before completing hosted acceptance or archive.
