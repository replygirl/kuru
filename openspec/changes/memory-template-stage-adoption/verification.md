# Verification

## 1. Adoption gives a copy its own independent identity [critical]

- [ ] 1.1 @integration (agent) run `adopted_copy_has_own_instance_scope_credentials_and_initial_revision` against two real Dolt engines on independently copied template-like stages -> distinct instance/secrets/heads per copy; the other copy's reader credential is refused; pre-adoption commit hashes equal
- [ ] 1.2 @integration (agent) run `adoption_requires_compiled_key_and_placeholder_on_both_refs` -> a foreign placeholder row on either ref fails with no commit made; a stale compiled key fails with a key error and no commit made

## 2. A verdict against the template's bytes is distinguishable from every engine failure [critical]

- [ ] 2.1 @integration (agent) run `adoption_verdicts_are_typed_and_engine_failures_are_not`, injecting a placeholder mismatch on each ref and a wrong UPDATE row count -> each produces Response::TemplateRejected, observed by the client as the distinct typed error, stage preserved
- [ ] 2.2 @integration (agent) same test, injecting a key mismatch, a bootstrap deadline, and a killed engine process -> each produces Response::Failed or a client-side startup error, never the typed verdict, stage preserved as today's failed-stage behavior

## 3. A copied stage a crash interrupts never starts an engine [critical]

- [ ] 3.1 @integration (agent) run `copy_remnant_without_identity_is_preserved_without_engine_start` against a real filesystem fixture, reading the engine ledger -> 0 engine starts recorded; stage ends under interrupted/
- [ ] 3.2 @integration (agent) run `unready_template_stage_is_preserved_without_engine_start` with hooks before S1, after the usage commit, after the main commit, and after initialized -> each hook point ends with 0 engine starts and the stage under interrupted/
- [ ] 3.3 @integration (agent) run `template_stage_is_classified_before_the_writable_recovery_start` -> a Class U shape is preserved with 0 starts, proving the check precedes the existing identity-without-marker writable-start branch

## 4. The template-shape check catches a replaced or non-empty tree before activation [critical]

- [ ] 4.1 @integration (agent) run `template_shape_violation_prevents_ready_marker` against a real stage engine, injecting an extra row, an extra commit and an extra branch one at a time -> each fails S1 with the typed verdict, stage preserved, no active directory produced

## 5. Cold stores and older binaries are unaffected

- [ ] 5.1 @unit (agent) run `cold_identity_record_bytes_are_unchanged` -> a cold-built store's identity.json bytes are byte-identical to before this change
- [ ] 5.2 @integration (agent) run `adopted_store_opens_under_a_later_key` -> a store adopted under compiled key K1 opens writable and read-only under a supervisor compiled to key K2 with zero key comparisons performed; a ready K1 template stage is reused by a K2-compiled binary through the existing inspection path
- [ ] 5.3 @unit (agent) run `ready_template_stage_is_reused` -> existing marker Before/After boundaries and reuse path activate a ready template-born stage unchanged, without re-running adoption

## 6. No caller regression: the template path stays unreachable in this change

- [ ] 6.1 @regression (agent) run `mise run //packages/kuru-memory:test` -> every existing test passes with its engine-start count unchanged, confirming Creation::Default and Creation::Cold behave exactly as before this change

## 7. Static checks

- [ ] 7.1 @unit (agent) run `mise run lint`, `mise run format:check`, `mise run typecheck` -> clean exit codes, including Windows-target lint on any new cfg-gated code
- [ ] 7.2 @manual (agent) run `mise run docs:check` after the docs passage lands -> passes, and the passage is read back to confirm it states no open path takes the template route yet
