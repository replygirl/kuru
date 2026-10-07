# Tasks

## 1. Inspect the retained native stage completely

- [x] 1.1 Add only a named nine-level scan and path explanation to the corrupt-image fixture's root; preserve all existing body and cleanup assertions.
- [x] 1.2 Run existing corrupt-image rejection, backup closing and named-depth guards, retaining beyond-budget rejection and no lowering of the default.
- [x] 1.3 Run affected host/Windows lint, typecheck, docs, format and managed checks, then strict/apply context review and actual archive before normal commit/push.

## Acceptance evidence

Original corrected-head Windows x64 job112604162345 reports checksum rejection,
then only the guarded root's depth-budget refusal for retained validation-stage
stats noms/temptf directories at depth nine. No corruption/source assertion
failure or native occupied-stage error is reported in this case.

Owning memory task87137 exited0: existing corrupt-image rejection, backup closing
scope and named-depth guard passed3/3, zero ignored, in4.26s. The depth guard
retains its expected caught rejection beyond its named budget and cannot lower
the default. Host49389, Windows38571, type91490, docs77171, format36904 and managed
checks each exited0. No production or global guard change was made.

Fresh native Windows execution, full own CI/required PR rollup, exact-main CI and
final release acceptance remain pending after publication. The separate old
updater-root OS32 failure has no identified locker and is not resolved by this
fixture adjustment.
