# Tasks

## 1. Identity marker

- [ ] 1.1 Add an optional `template: Option<String>` field to `Identity` in
      `server.rs` with `#[serde(default, skip_serializing_if = "Option::is_none")]`,
      keeping `deny_unknown_fields`, and verify by a unit test asserting a
      cold-built store's serialized `identity.json` bytes are unchanged
      (`cold_identity_record_bytes_are_unchanged`).
- [ ] 1.2 Add `pub(crate) fn stage_template_key(directory: &Path) -> Result<Option<String>>`
      in `server.rs` that reads `identity.json` through the same checked
      reader and struct `load_identity` uses, without a second struct, and
      verify by a unit test reading a hand-written template-marked
      `identity.json` and a hand-written cold `identity.json` and asserting
      `Some`/`None` respectively, plus a parse-failure case returning an
      error.

## 2. Adoption in the Dolt bootstrap

- [ ] 2.1 Add the compiled-placeholder-key comparison at the top of the
      `!identity.initialized` branch of `initialize_database`, behind a
      single function so it can be replaced when P4b supplies the real key
      computation, running for both a build identity (instance =
      `TEMPLATE_INSTANCE`, not exercised by this change's callers but not
      excluded by the check) and a copy identity (`template` set, instance
      differs from `TEMPLATE_INSTANCE`), and verify by
      `adoption_verdicts_are_typed_and_engine_failures_are_not`'s key-mismatch
      case: a stale/foreign compiled key yields `Response::Failed` naming
      both keys, with no adoption SQL executed.
- [ ] 2.2 Add the adoption group: gated on `!identity.initialized && identity.template.is_some()`,
      on one acquired connection, `USE` the usage branch, verify the
      placeholder instance/scope row, `UPDATE` it to the new identity with a
      guard clause on the placeholder values, assert exactly one row
      affected, `CALL DOLT_COMMIT('-am', ..., '--author', AUTHOR)`; then the
      same four steps on `main`; then the existing `kuru_reader` creation and
      row-verification extended to check the adopted row on both refs before
      `initialized: true` is written. Verify by
      `adopted_copy_has_own_instance_scope_credentials_and_initial_revision`:
      two independently adopted copies of the same template-like source end
      with distinct instance, secrets and heads, the other copy's reader
      secret is refused, and pre-adoption commit hashes are equal between the
      two copies.
- [ ] 2.3 Add `Response::TemplateRejected(String)` beside `Response::Failed`
      in the `Response` enum (`deny_unknown_fields` preserved), and wire
      `supervisor_request` to send it only when the bootstrap error downcasts
      to the adoption group's own verdict type (placeholder row mismatch on
      either ref before rewrite, or an affected-row count other than one),
      mapping every other bootstrap error — including the key mismatch from
      2.1 — to `Response::Failed`. Verify by
      `adoption_verdicts_are_typed_and_engine_failures_are_not`'s full matrix:
      a placeholder mismatch on either ref and a wrong `UPDATE` count each
      produce `Response::TemplateRejected`; a key mismatch, an injected
      bootstrap deadline and a killed engine each produce `Response::Failed`
      or a client-side startup error; the client-side discriminant the caller
      observes matches which bucket the server reported.

## 3. Shared template-shape check

- [ ] 3.1 Write one function, parameterized by the expected identity row and
      by branch-set/commit-count expectations derived from the compiled
      migration registries (never hardcoded), that checks: the branch set
      equals main plus the usage branch plus the retained migration
      branches; every retained branch is clean and classified as published;
      working sets are clean; every project-data table is empty on main and
      the usage branch; views, triggers, procedures and `dolt_ignore` rules
      are empty or absent on both refs; the instance row equals the expected
      identity on both refs; and commit counts on both refs equal the
      registry-derived expectation. Verify by unit test
      `template_shape_counts_derive_from_registries` (synthetic registries,
      asserting the formula, not a fixed number) and by exercising it from
      the S1 call site in task 3.2.
- [ ] 3.2 Call that function from the adoption path on the stage engine after
      the commits of task 2.2 and before `ready.json`, with the adopted
      identity and the post-adoption commit counts as its expected values,
      and treat a failing query result as the typed verdict of task 2.3 while
      a query error or timeout stays an ordinary failure. Verify by
      `template_shape_violation_prevents_ready_marker`: an extra row, an
      extra commit and an extra branch, injected one at a time into an
      otherwise-adopted stage, each fail S1 with the typed verdict, leave the
      stage preserved, and produce no active directory.

## 4. Recovery classes R and U

- [ ] 4.1 In `recover_staging` (`store.rs`), before the existing branch that
      starts a writable engine on an identity-without-marker stage
      (store.rs:7443-7454), add Class R: no `identity.json`, `data/` present,
      and every top-level entry in `{data/, staging/ holding only temporary
      record files, lifecycle.lock}`; wait for quiescence within the existing
      bound, then preserve under `interrupted/` with no engine start. Verify
      by `copy_remnant_without_identity_is_preserved_without_engine_start`:
      the engine start ledger shows 0 starts for the stage and it ends under
      `interrupted/`.
- [ ] 4.2 Add Class U immediately after: `identity.json` present with
      `stage_template_key` returning `Some` and no `ready.json`; wait for
      quiescence within the existing bound, then preserve under
      `interrupted/` with no engine start and no adoption SQL run from
      recovery. Verify by
      `unready_template_stage_is_preserved_without_engine_start` with hooks
      before S1 starts, after the usage-branch commit, after the main-branch
      commit, and after `initialized` is set but before `ready.json` — each
      hook point ends preserved with 0 starts — and by
      `template_stage_is_classified_before_the_writable_recovery_start`
      proving Class U's check precedes the existing store.rs:7443-7454
      branch.
- [ ] 4.3 Confirm `ready_template_stage_is_reused` needs no new code: a
      template-born stage that already reached `ready.json` follows today's
      existing-ready-stage reuse path unchanged (marker Before/After
      boundaries, `validate_ready`, rename). Verify by that test asserting
      the marker boundaries and that the existing reuse path activates the
      stage without re-running adoption.
- [ ] 4.4 Update `store/recovery_tests.rs`'s "unrecognized interrupted import
      without server identity" test so it covers only entries that remain
      genuinely unknown after classes R and U are added. Verify: that test
      still fails closed on a fabricated unknown shape and no longer treats a
      Class R or Class U shape as unrecognized.

## 5. Cross-cutting adoption test

- [ ] 5.1 Add `adopted_store_opens_under_a_later_key`: a store adopted under
      compiled key K1 opens writable and read-only under a supervisor
      compiled to a different key K2 (proving the "only while `!initialized`"
      rule of task 2.1/AGENTS decision), and a ready K1 template stage is
      reused by a K2-compiled binary through the existing inspection path.
      Verify by both opens succeeding with no key comparison performed
      (assert via an injected key-comparison call counter fixed at zero).

## 6. Docs

- [ ] 6.1 Add a short passage to `docs/memory.md` and
      `apps/kuru-docs/concepts/memory.md` on template-born stores' identity
      marker and recovery classes, stating plainly that no open path takes
      this route yet and nothing user-visible changes. Verify: `mise run
      docs:check` passes and the passage names the `template` field, the
      typed verdict, and classes R/U without promising a creation path that
      does not exist yet.

## 7. Full verification

- [ ] 7.1 Run `mise run //packages/kuru-memory:test` and confirm all ten P4a
      tests (section 9 of the design) plus the updated existing tests pass,
      and confirm no other test's engine-start count changed (nothing yet
      selects the template path). Verify: recorded command output showing
      the new tests passing and the full package suite green.
- [ ] 7.2 Run `mise run lint`, `mise run format:check` and `mise run
      typecheck` (or the equivalent package-scoped `//packages/kuru-memory:*`
      addresses) and confirm clean, including Windows-target lint on the new
      `cfg`-gated code if any. Verify: each command's exit code and a note of
      any warnings addressed.
