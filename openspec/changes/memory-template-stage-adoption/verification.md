# Verification

Local evidence: macOS 27.0 arm64, debug profile, pinned Dolt 2.3.5, run with
`mise run //packages/kuru-memory:test -- --lib template_` (38.8 s for the ten
engine tests) on the implementation commit, and again after the review fixes
(18 tests matching `template_`, 0 failed, 43.2 s). Linux and Windows evidence
comes only from CI on the pull request; none is claimed here.

## 1. Adoption gives a copy its own independent identity [critical]

- [x] 1.1 @integration (agent) run `adopted_copy_has_own_instance_scope_credentials_and_initial_revision` against two real Dolt engines on independently copied template-like stages -> distinct instance/secrets/heads per copy; the other copy's reader credential is refused; pre-adoption commit hashes equal. Observed: passed; each adoption commit's sole parent is the template head, retained branch heads equal the template's, the initial revision is the `main` adoption head, and both root and reader secrets of the other copy are refused with MySQL 1045.
- [x] 1.2 @integration (agent) run `adoption_requires_compiled_key_and_placeholder_on_both_refs` -> a foreign placeholder row on either ref fails with no commit made; a stale compiled key fails with a key error and no commit made. Observed: passed; heads read from a served copy of each refused stage equal the source's on every branch, and the identity stays uninitialized.

## 2. A verdict against the template's bytes is distinguishable from every engine failure [critical]

- [x] 2.1 @integration (agent) run `adoption_verdicts_are_typed_and_engine_failures_are_not`, injecting a placeholder mismatch on each ref and a wrong UPDATE row count -> each produces Response::TemplateRejected, observed by the client as the distinct typed error, stage preserved. Observed: passed; in-process (Unix) frames were `TemplateRejected` for a foreign usage row, a foreign main row, a dirty working set and a zero-row rewrite; through the spawned supervisor a foreign row is a client `TemplateVerdict`. The refused stage stays unready in place for the next open's Class U preservation (see design.md).
- [x] 2.2 @integration (agent) same test, injecting a key mismatch, a bootstrap deadline, and a killed engine process -> each produces Response::Failed or a client-side startup error, never the typed verdict, stage preserved as today's failed-stage behavior. Observed: passed; in-process frames were `Failed` for the key mismatch (naming both keys), a stall after the usage commit ending at the 15 s bootstrap deadline ("deadline exceeded while adopting the store template on the usage branch") and a SIGKILL of the owned engine after the usage commit ("failed while adopting the store template on main"); through the spawned supervisor the key mismatch and a missing usage branch (SQL error) are not verdicts; each stage stays uninitialized.

## 3. A copied stage a crash interrupts never starts an engine [critical]

- [x] 3.1 @integration (agent) run `copy_remnant_without_identity_is_preserved_without_engine_start` against a real filesystem fixture, reading the engine ledger -> 0 engine starts recorded; stage ends under interrupted/. Observed: passed for a bare remnant and one with an interrupted identity write's temporary record.
- [x] 3.2 @integration (agent) run `unready_template_stage_is_preserved_without_engine_start` with hooks before S1, after the usage commit, after the main commit, and after initialized -> each hook point ends with 0 engine starts and the stage under interrupted/. Observed: passed; one open preserved all four stages with no start added by recovery. The two mid-adoption hooks are Unix-only (in-process supervisor); CI's Windows jobs run the other two.
- [x] 3.3 @integration (agent) run `template_stage_is_classified_before_the_writable_recovery_start` -> a Class U shape is preserved with 0 starts, proving the check precedes the existing identity-without-marker writable-start branch. Observed: passed; the same stage without the `template` field takes the writable recovery start (1 start, identity mismatch). Mutation check: forcing classes R and U off failed this test and the two above.

## 4. The template-shape check catches a replaced or non-empty tree before activation [critical]

- [x] 4.1 @integration (agent) run `template_shape_violation_prevents_ready_marker` against a real stage engine, injecting an extra row, an extra commit and an extra branch one at a time -> each fails S1 with the typed verdict, stage preserved, no active directory produced. Observed: passed, and a view as a fourth case; the placeholder form of the same check runs in every test's template build and passed. After review, a fifth case adds 257 empty tables to the usage branch: the check reads one row past its 256-table bound and reports "more than 256 tables" as a verdict. Mutation check: ignoring that bound let the stage publish `ready.json` ("table overflow: the stage was marked ready"), because a truncated table list left the remaining tables unchecked. The branch list uses the same one-past-the-bound read.

## 5. Cold stores and older binaries are unaffected

- [x] 5.1 @unit (agent) run `cold_identity_record_bytes_are_unchanged` -> a cold-built store's identity.json bytes are byte-identical to before this change. Observed: passed; a record type without the field refuses a template-born record, as an older binary does. After review, `stage_template_key_reads_the_identity_record` also refuses keys that are not one portable path component (uppercase, `/`, `\`, `..`, `.`, space, non-ASCII) and accepts a 64-character hex key and a key at the 128-byte bound.
- [x] 5.2 @integration (agent) run `adopted_store_opens_under_a_later_key` -> a store adopted under compiled key K1 opens writable and read-only once its identity names another key, and a ready template stage naming another key is reused through the existing inspection path. Observed: passed; the marker is not rewritten by either open. A failed comparison would refuse the open, so no separate counter was added.
- [x] 5.3 @integration (agent) run `ready_template_stage_is_reused` -> existing marker Before/After boundaries and reuse path activate a ready template-born stage unchanged, without re-running adoption. Observed: passed; after the After boundary the open activates the same directory at its initial revision with one adoption commit (`main`'s length equals the compiled registries' count plus the adoption commit, derived by `template_shape::compiled_commits`; 9 at schema 7) and one inspection plus one active start.

## 6. No caller regression: the template path stays unreachable in this change

- [x] 6.1 @regression (agent) run `mise run //packages/kuru-memory:test` -> every existing test passes with its engine-start count unchanged, confirming Creation::Default and Creation::Cold behave exactly as before this change. Observed on macOS: exit 0; library 392 passed, 0 failed, 4 ignored; integration targets 10, 5, 12 and 1 passed; the existing budget and start-count tests are unchanged and passed.

## 7. Static checks

- [x] 7.1 @unit (agent) run `mise run lint`, `mise run format:check`, `mise run typecheck` -> clean exit codes, including Windows-target lint on any new cfg-gated code. Observed: `//packages/kuru-memory:lint`, `//packages/kuru-memory:lint:windows` and `//packages/kuru-memory:typecheck` exited 0; `format:check` passed.
- [x] 7.2 @manual (agent) run `mise run docs:check` after the docs passage lands -> passes, and the passage is read back to confirm it states no open path takes the template route yet. Observed: exited 0; both passages say no open creates a store from a template yet.
