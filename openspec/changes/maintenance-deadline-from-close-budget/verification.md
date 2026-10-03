# Verification

## 1. Maintenance outlasts a close held past the startup timeout [critical]

- [ ] 1.1 @regression (agent) `service::tests::maintenance_behind_a_close_held_past_the_startup_timeout_acquires_within_the_close_budget` on the test commit without the fix -> fails at the 30 s deadline
- [ ] 1.2 @regression (agent) the same test with the fix, three times -> passes

## 2. Existing deadline behaviour

- [ ] 2.1 @integration (agent) maintenance, purge and retirement tests -> pass
- [ ] 2.2 @integration (agent) full `mise run //packages/kuru-memory:test` -> exit 0

## 3. Static checks

- [ ] 3.1 @unit (agent) format:check, lint, lint:windows, typecheck, docs:check, cospec validate --all --strict -> each exit 0
- [ ] 3.2 @runtime (agent) native PR CI -> defer
