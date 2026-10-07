# Tasks

## 1. Compare persisted session state

- [x] 1.1 Preserve the existing read-only fixture, explicitly validate presence/driver types, remove only ephemeral known-presence from both captured listings and compare every remaining field exactly.
- [x] 1.2 Run the exact inspection case and existing closing guard plus host/Windows lint, typecheck, formatting and managed checks.
- [x] 1.3 Record actual evidence, strict/apply contexts, actual archive and normal commit/push; final-head full CI/PR/main remain required.

## Acceptance plan

Original macOS112618132715 failed raw sessions equality because presence_known
was false then true; all durable fields/driver null matched. The approved honest
bounded live-presence contract allows this transition without catalog mutation.
Corrected local evidence follows; fresh final-head native/full CI remains due.

## Observed evidence

Owning43933 exited0: exact real CLI inspection passed1/1 zeroignored11.59s and
existing application closing guard passed1/1 zeroignored0.03s. Host63130,
Windows51338, type87622, format30517 and managed checks each exited0. Frozen
11+/1- fixture diff was independently reviewed; all other body assertions and
production bytes remain unchanged. Fresh full CI/PR rollup/exact-main/release
remain required, not inferred from local acceptance.
