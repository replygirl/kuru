# Tasks

## 1. Snapshot fixture isolation

- [x] 1.1 Add one private fixture mutex and poison-recovering guard helper in packages/kuru-platform/tests/unix_snapshot.rs; acquire the guard before setup/deadline creation in every fixture, without changing the library's deliberate contention controls.
- [x] 1.2 Verify every original native API call, assertion, deadline and cleanup statement is preserved, including the stalled helper's original 100 ms deadline and required PID marker plus owned-row absence. Remove the temporary retry adapter and its controls.
- [x] 1.3 Run the affected platform snapshot target once without competing local checks, then scoped platform formatting/lint/typecheck and strict/apply/managed checks; record actual local results and preserve required fresh native PR CI before merge as unrun.

Acceptance is authored before this implementation. Actual Ubuntu/macOS CI
failures at `77e49b87` are negative evidence. The historical lock holder and exact
scheduler interleaving remain unknown. A local target run with the temporary
retry adapter failed its missing-helper-marker assertion; no failed run is
reported as a pass. No production code, budget, workflow, dependency or coverage
requirement changes. The original archived Unix change remains untouched.

Mandatory closeout after local acceptance: execute actual Cospec archive,
confirm the active directory is absent and follow-up archive present, and make
the branch commit through normal hooks. Report actual archive/commit results,
never force incomplete tasks or claim hosted success. Root owns publication and
fresh native/instrumented CI before merge.

## Observed local acceptance (2026-10-05)

- The final source adds only the mutex import, private mutex/guard helper and
  one guard at the start of each of the 13 original fixtures (11 on macOS).
  Removing those additions reproduces the `77e49b87` file byte-for-byte; all
  original calls, assertions, cleanup and deadlines remain unchanged. The
  production source and original archived Unix record have no diff.
- `KURU_MBX=0 mise exec -- cargo test -p kuru-platform --test unix_snapshot
  --all-features --locked -- --nocapture`: exit 0, 11 passed, 0 failed, 3.08 s
  on macOS. No competing local checks ran during this target. Both fixtures
  that failed in native CI passed, including the actual helper PID marker and
  absence from owned process rows after the original 100 ms deadline.
- Platform `lint`, `typecheck` and `format:check` mise tasks: exit 0 each.
  `mise run cospec:managed:check`: exit 0, no drift. `git diff --check`: exit 0.
  The final-scope strict validation and actual apply gate both exited 0;
  validation reported 0 errors/warnings and apply reported clear.
- Negative evidence is the two actual native instrumented failures at
  `77e49b87`, plus the local temporary retry version's 12 passed/1 failed
  target (exit 101, required helper PID marker absent). The temporary adapter
  and its injected controls are not part of this change. No exact historical
  lock-holder reproduction or new physical ECHILD check is claimed.
- Fresh native Linux/macOS instrumented coverage and Windows acceptance for
  this correction remain unrun locally and required through PR CI before merge.
  The existing 90% workspace coverage gate remains unchanged.

Source review accepted the final mutex-only scope and local evidence. Actual
archive and normal-hook commit remain mandatory closeout actions after these
completed local tasks; their execution is recorded below when performed.

## Executed archive closeout

`mise run cospec -- archive unix-snapshot-admission-tests` exited 0 and reported:

```text
Archived: unix-snapshot-admission-tests (test) → openspec/changes/archive/2026-10-05-unix-snapshot-admission-tests/
Specs:    skipped
```

The active directory is absent and the named archive exists. No living spec or
original archived change was modified. `mise run cospec -- validate --all --strict` and
`mise run cospec:managed:check` both exited 0 after archive; strict validation
reported 0 errors/warnings and managed checking reported no drift. The final
normal-hook commit remains mandatory; its exact hash and hook outcome are
reported separately after execution.
Fresh hosted native/instrumented acceptance remains required before merge.
