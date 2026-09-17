## 1. Built-in mode dispatch remains equivalent [critical]

- [x] 1.1 @runtime (agent) Run deterministic target, focus, activation, continuity, cold-tie, peer, relationship, and consultation scenarios for all four built-ins -> normalized requests, deliveries, contributions, speaker/reason, limits, events, and durable outcomes match the approved pre-extraction baseline.
- [x] 1.2 @integration (agent) Run real-Dolt completed, cancelled, and exact-retry turns through policy-routed dispatch -> budgets, cancellation, persistence-before-publication, P6 denials, P7 accounting, and no-replay behavior remain intact.

## 2. Policy seams reject or change actual runtime effects [critical]

- [x] 2.1 @integration (agent) Inject alternate Roles, Peering, Flow, and Facing policies through the internal harness fixture -> changed seeds, recipient/speaker decisions, and restrictions affect runtime behavior without a shipping mode or loop edit.
- [x] 2.2 @runtime (agent) Deny a direct peer edge and suppress a consultation with test-only policies -> no mail, event, provider request, delivered tool result, or tool side effect occurs before the truthful refusal.
- [x] 2.3 @integration (agent) Return inactive, nonpending, invalid-contribution, and ineligible-speaker policy outputs -> each rejects before external dispatch and leaves durable topology unchanged.

## 3. Relationship origin and dream role checks remain safe [critical]

- [x] 3.1 @runtime (agent) Create valid relationships from direct User/API and participating Peer origins, then attempt nonparticipating, self, inactive, and invalid-member proposals -> valid canonical relationships retain behavior and invalid proposals have no publication or durable mutation.
- [x] 3.2 @integration (agent) Exercise dream proposal, retirement, undo, and candidate rejection with a test role profile -> required-role violations fail before promotion while candidate isolation and later-history-preserving undo remain intact.

## 4. Repository and boundary review

- [x] 4.1 @regression (agent) Run scoped core/runtime format, typecheck, lint, and focused golden/real-Dolt tests after P7 is integrated -> affected package checks pass with P6/P7 behavior preserved.
- [x] 4.2 @equivalence (agent) Independently review the engine and core diff against this change, P6, P7, and P9 ownership boundaries -> no policy gains execution, memory, visibility, publication, or supervisor authority.
- [x] 4.3 @eval (agent) Run scripted-provider four-mode parity scenarios with test-only policy overrides -> normalized request and event inventories demonstrate each dispatch seam changes only its approved behavior.
- [x] 4.4 @regression (agent) Run documentation checks and one combined instrumented workspace coverage suite -> documentation builds and links pass, all behavioral checks pass, and workspace line coverage remains at least 90% without exclusions.

Native CI and release verification remain delivery requirements after local archive and before merge/publication. Record their actual run links during PR review; this ledger does not claim an unrun remote pass.

## Observed local evidence

- Pre-extraction baseline at parent `aec1bbf`: all three `mode_baseline_tests`
  passed across the four modes with real Dolt, before dispatch source changes.
- Core package tests: 54 passed, including the five policy tests and unchanged
  four-mode seed-byte goldens. Core typecheck, lint and formatting passed.
- Runtime package typecheck passed for all targets after dispatch integration.
- `mise run docs:check` passed in 3.79 seconds, including the public build and
  content/link/anchor checks. Local log:
  `/private/tmp/kuru-phase1-mode-dispatch-docs.log`.
- Post-extraction four-mode baseline: 3/3 passed with the same expected requests
  and outcomes. Two old stable-ID fixtures now create their nonseed parts as
  live actors before selecting them; expected tie-break results are unchanged.
- Alternate-profile real-Dolt tests: 7/7 passed. They exercise seeds and role
  retirement/reopen/undo, actual initial and later recipient selection, retained
  pending input, contribution omission and forgery, speaking selection and
  explicit-target protection, delivery denial, consultation suppression,
  malicious relationship substitution, and a P6-denied file write with no file
  effect under an alternate Facing policy.
- The focused accepted-write/publication/reconcile test passed 1/1. The old
  authored-tie control passed 1/1 with live nonseed actors. Runtime typecheck,
  Clippy, Rust formatting and diff checks passed.
- Independent engine/dream review found no remaining policy-enforcement or
  publication bypass. Root's relationship-participation finding was corrected
  with runtime proposal/result validation and an adversarial fixture. Core
  origin and profile boundaries were reviewed alongside the unchanged built-in
  fixtures.
- Combined workspace coverage passed at 93.77% (44,767/47,740 lines), with exit 0
  in 642.97 seconds, including
  all 129 runtime tests and the existing permission, accounting, cancellation,
  exact-retry, dream candidate, undo, CLI and terminal controls. Log:
  `/private/tmp/kuru-phase1-mode-dispatch-coverage.log`; retained LCOV:
  `/private/tmp/kuru-phase1-verification/mode-dispatch-coverage.lcov`.
- Final aggregate `mise run format:check` passed in 2.97 seconds, and strict
  Cospec validation passed with zero errors or warnings. No native CI,
  publication, merge or release result is claimed by these local checks.
