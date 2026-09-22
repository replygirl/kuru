## 1. Exact owner and pool shutdown [critical]

- [x] 1.1 @regression (agent) hold one real-Dolt pool connection through the first close deadline, observe exact child reap through a test-only one-shot, then release the dead client socket -> pre-fix close reports `memory pool close deadline exceeded`; corrected close finishes the same closed pool and succeeds within the second named deadline
- [x] 1.2 @integration (agent) keep the held connection past both deadlines -> close reports an actionable post-reap pool-drain failure after the exact owner is reaped
- [x] 1.3 @integration (agent) run the existing in-flight-disconnect fixture -> query and SQL-session teardown remain required before shutdown

## 2. Affected checks and native evidence

- [x] 2.1 @regression (agent) run package-owned memory format, lint, typecheck and focused native fixtures -> corrected source passes
- [~] 2.2 @integration (agent) run normal combined coverage and hosted Intel macOS exact-head memory suite -> defer: normal push and hosted CI follow the reviewed commit

Pre-fix evidence, 2026-09-22: the first version of `post_reap_pool_drain_finishes_a_returned_real_connection` ran against the unmodified shutdown code with a current prepared supervisor and real Dolt. It held one pool connection through the first deadline, used lifecycle-lease acquisition as its reap signal, released the connection, and failed 1/1 in 9.55 seconds with `memory pool close deadline exceeded`. A contract audit showed that lifecycle authority ends at exact Dolt reap, while explicit close separately awaits pool cleanup. The final fixture therefore uses a test-only exact-reap one-shot; it does not claim the earlier lease observation as proof of a lock held through pool drain. Its first sandboxed attempt failed earlier on loopback bind permission and is not product evidence. Hosted #56 Intel macOS originally showed `closed=true,size=1,idle=1` at the eight-second close deadline after the SLEEP SQL session had already ended.

Post-fix native macOS evidence, 2026-09-22: `cargo test -p kuru-memory --lib post_reap_pool_drain --locked` passed 2/2 real-Dolt fixtures in 18.07 seconds with the current prepared supervisor. The positive fixture observed exact owner reap, released the held client, and required `close` success, pool size zero and quiescence. The negative held its client through both deadlines and required the named post-reap timeout before releasing and draining it. `in_flight_disconnect_waits_for_real_query_and_session_teardown` then passed 1/1 in 1.61 seconds. The first post-fix compile found only a missing test-only observer field in three `Owner` fixture constructors; those are corrected in this source.

Affected checks, 2026-09-22: package-owned `//packages/kuru-memory:typecheck` and `//packages/kuru-memory:lint` passed with all targets/features; `cargo fmt --all -- --check` and `git diff --check` passed. The root `format:check` aggregate stopped in the unrelated TOML formatter because its macOS system-configuration dynamic-store call panicked in this sandbox; no TOML file changed in this fix. Hosted Intel CI remains pending normal delivery.
