# Design

## Context

This is P4a of the per-machine store template design
(`tmp/roadmap/store-creation-design-machine-cache-2026-09-29.md`, revision 2,
FIX_THEN_ACCEPT). The full design lets a new project be created by copying a
pre-migrated template's `data/` into an ordinary stage and starting one engine
there (P4b, not part of this change). This change lands only the receiving
side that stage needs: the identity marker, one-shot adoption in the Dolt
bootstrap, the typed verdict distinct from an ordinary failure, the two
recovery classes a crash can leave behind in a copied stage, and the
template-shape check both the future build and this stage engine will run.
Order 1's prerequisites (P1 engine spike, P2 main-pool classification, P3
stage worker extraction) are already archived on this tree
(`blocking-changes.md`). No caller of this path exists yet: `Creation::Default`
and `Creation::Cold` are unchanged, and P4a's own tests construct template-like
stages by hand (a stopped fixture store's `data/` copied by the checked
filesystem helpers, plus a hand-written `identity.json`) rather than through a
builder, because no builder exists until P4b.

The adoption protocol's engine assumptions are not speculative: PR1's spike
(`tmp/roadmap/store-creation-design/engine-contract-findings.md`, archived as
`2026-09-29-memory-engine-contract-tests`) measured S1 (root authenticates
from `DOLT_ROOT_PASSWORD` on copied data with an empty `config/`; the
template's own root and reader secrets are refused with MySQL 1045) and S2
(one connection can `USE`, run a guarded `UPDATE` and `DOLT_COMMIT('-am', ...)`
on the usage branch, then the same on main; each new head's only parent is the
template head; a pool on a retained migration branch fails the per-connection
identity check after adoption) against real Dolt 2.3.5, and
`packages/kuru-memory/src/store/engine_contract_tests.rs` already exercises
this SQL by hand.

## Goals / Non-Goals

**Goals:** see `proposal.md`'s What Changes. In short: the `template` identity
field and its backward-compatible serialization; one-shot bootstrap adoption
under a compiled-placeholder comparison; the typed verdict response,
distinguished from every engine/SQL/I/O/deadline failure and from a
supervisor key mismatch; recovery classes R (copy remnant) and U (unready
template stage), both preserved without an engine start; one shared
template-shape-check function for later reuse by the P4b build.

**Non-Goals:** no template cache, key computation, build worker, copy worker
or creation-path selector (P4b) — no open takes the template path after this
change. No quarantine of a template (P4b reports it; this change only defines
the verdict it would be quarantined on). No restart removal (P5), no
migration-publication records (P6), no change to timeouts, deadlines,
retries, or the cold path's own behavior.

## Decisions

- **The `template` field is permanent for the store's life, not cleared after
  `ready.json`.** Rejected alternative: drop it once `ready.json` is written.
  That would leave a window, between adoption completing and the marker being
  removed, in which a crash produces a stage indistinguishable from an
  ordinary writable-recovery candidate — exactly the ambiguity AGENTS.md
  forbids resolving by inference ("never promote, replay or abandon a
  candidate from missing or ambiguous evidence"). Keeping the marker gives
  Class U a durable signal and costs only that an older binary, which already
  fails closed on any unrecognized identity field, also fails closed on a
  template-born store; that is the documented, intended trade (design
  section 4.1).

- **The key comparison runs only while `!initialized`, never again after.**
  Rejected alternative: keep comparing the marker against the compiled key on
  every bootstrap. Because the marker is permanent (previous decision), that
  would make an already-adopted, fully independent project fail to open after
  any later release changes the template key (a schema step, a keyed creation
  statement, or an engine version bump) — the marker would otherwise need to
  be mistaken for ongoing provenance of the *running* engine rather than a
  one-time fact about *how the store was born*. Comparing only pre-adoption
  keeps the guarantee where it matters (the supervisor performing adoption is
  the one that built or trusts the template) without coupling every future
  open to every future key.

- **A verdict is only a completed comparison that returns a different value;
  everything else — including every engine, SQL, I/O and deadline failure,
  and a key mismatch — is an ordinary failure.** This is the design's B1
  disposition, carried into P4a because the verdict type this PR introduces
  is exactly what the future quarantine reads. Rejected alternative: treat
  any failure during adoption as suspect. That would (per the design's
  rejected review finding) let an unrelated engine crash or timeout condemn
  a perfectly good template once P4b's quarantine consumes this verdict type,
  which is cross-request, cross-project state riding on a transient fault.
  Getting the discriminant right here, before any quarantine code exists,
  means P4b only has to consume a already-correct signal.

- **A key mismatch is its own outcome, not a verdict and not folded into the
  generic engine-failure bucket silently.** It says something real (this
  supervisor was not built to trust this template) but says nothing about the
  bytes, so it must stay `Response::Failed` while still being distinguishable
  enough in the error text (naming both keys) for diagnosis. Rejected
  alternative: a third response variant for key mismatches specifically.
  Not taken here because nothing downstream (this change has no quarantine
  logic) needs to distinguish it programmatically from any other ordinary
  failure; the design records this as future work only if P4b's error
  handling needs it.

- **One shared shape-check function, parameterized by the expected identity
  row and the registry-derived branch/commit-count expectations, rather than
  two separate checks for "build assertions" and "S1 shape check".** A bug in
  the check then fails in one place and is caught by whichever caller runs
  first in tests, instead of two implementations silently drifting and one of
  them passing a tree the other would have rejected. This PR only has the S1
  caller (adoption's own post-commit verification); P4b's build-time caller
  reuses the same function unchanged.

- **Classes R and U are checked before the existing identity-without-marker
  writable-recovery branch, never after.** A stage with `identity.json`
  present, `template` set and no `ready.json` would otherwise be picked up by
  today's generic "identity exists, no marker" recovery, which starts a
  writable engine on it — exactly the "adoption in a recovery start" defect
  the design's RG N5 disposition forbids. Ordering the new classes first, and
  giving neither of them an engine start, is the only way to keep "no engine
  starts on an unready template stage" true without special-casing the
  generic branch itself.

- **Adoption verifies both refs before rewriting either.** The usage branch
  and `main` must each hold a clean working set and the compiled placeholder
  as their only identity row before the first guarded rewrite. Rejected
  alternative: verify-then-rewrite per ref, as the design's step list reads.
  A placeholder foreign only on `main` would then leave an adopted usage
  branch behind a refused stage, and `DOLT_COMMIT('-am')` would commit any
  dirty template working set into the adoption commit. The spec delta's
  "before rewriting either one" requires the stricter order.

- **A failed adoption start leaves the stage where it is.** The stage
  engine's reap guard is the project's startup lock; a failed
  `Server::open_with_guard` releases it with the reaped owner. Preserving the
  stage then, without the lock, could race another opener, so the adopt job
  returns the error and leaves the unready stage for the next open's recovery,
  which preserves it as Class U without a start, exactly as a failed first
  staging start is left today. A failure after a successful start (validation
  or the shape check) is preserved by the job itself, under the returned
  lock, as `validate_and_mark` does.

- **The adopt job is a stage-worker job with no product caller yet.**
  `StageWorker::adopt_and_mark`, the shape check module and the two identity
  writers are production code carrying `expect(dead_code, reason = ...)` only
  outside tests, so the template cache's creation path adds a caller and
  removes the expectation instead of moving test code into production.

- **The shape check classifies retained main attempts only.** Main attempts
  are classified from `main` as every open does. Classification compares a
  retained attempt's head with the head of the ref it serves, so usage
  attempts are classified by `validate_usage` from the usage pool (which the
  build and every writable open run); the shape check only requires their
  count and targets to match the registry.

- **Engine faults inside adoption are exercised in an in-process
  supervisor on Unix only.** A task-local fault (`server::adoption_fault`)
  can fail, stall or kill the owned engine at a point in the adoption group
  and can make the usage rewrite change zero rows. The in-process supervisor
  pattern exists only in the Unix server tests (Windows supervisors use a
  private pipe rendezvous). Windows still runs every test that goes through
  the spawned supervisor: adoption, both verdict directions at the client,
  classes R and U before the first start and after initialization, reuse,
  the later key and the shape verdicts.

## Risks / Trade-offs

- [Risk] The shared shape-check function is exercised in this change only
  through the S1 adoption path (no build worker exists yet), so a bug that
  only the build-time caller's parameterization would trigger stays latent
  until P4b. → Mitigation: the function is written parameterized from the
  start (expected identity row and registry-derived counts as arguments, not
  hardcoded S1 values), and P4a's own test
  `template_shape_violation_prevents_ready_marker` exercises the S1 call site
  against deliberately wrong rows, commit counts and branch sets, which is
  the same code path P4b's build assertions will call.

- [Risk] `deny_unknown_fields` on `Identity` means any future binary that adds
  its own optional identity field without `skip_serializing_if` discipline
  reintroduces the same forward-compatibility failure this PR is careful to
  avoid for `template`. → Mitigation: not a new risk this change introduces,
  but the spec delta states the byte-identical-serialization requirement
  explicitly (`cold_identity_record_bytes_are_unchanged`) so a regression
  here is an observable, tested contract, not an implicit assumption.

- [Risk] Because no caller selects the template path yet, every test in this
  change constructs its fixture stage by hand rather than through a real
  copy, so a defect specific to the P4b copy worker's own file-handling
  (partial writes, sync ordering, symlink handling) cannot be caught here.
  → Mitigation: explicitly out of scope (non-goal); the design's section 9
  already separates these into P4a's ten tests versus P4b's, and P4b's own
  tests (T11 `copied_files_and_directories_are_synced_before_identity`, T8
  `template_byte_corruption_mid_copy_preserves_remnant_and_quarantines`) cover
  that surface when the copy worker exists.
