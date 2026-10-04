# Verification

Rows are filled with observed evidence as checks finish; none is claimed before it is run.

## 1. Bounds end on events under the shard deadline

- [ ] 1.1 @regression (agent) a unit test where a command or settle outlives the old literal but not the shard deadline fails before the fix (literal expiry decides) and passes after (shard deadline decides) -> pending
- [ ] 1.2 @unit (agent) tests pin each derivation (constant derives from the shard deadline chain or existing product budget; no `SERVICE_IDLE_TIMEOUT` citation remains) -> pending
- [ ] 1.3 @unit (agent) the three former thin-race inline tests in `coverage.rs` wait on observed events and assert the same outcomes -> pending
- [ ] 1.4 @integration (agent) `mise run //packages/kuru-delivery:test` passes -> pending
- [ ] 1.5 @integration (agent) `mise run //packages/kuru-delivery:lint` and `lint:windows`, `format:check`, `typecheck`, `docs:check` pass -> pending
- [ ] 1.6 @runtime (agent) the release workflow `tests` job records its job start and a launch there is bounded by the job deadline -> defer: observable only on a release dispatch; verified by workflow review and the workflow checks
- [ ] 1.7 @runtime (agent) native partitions on every platform and the open-time gate pass on the final head with no rerun -> pending
