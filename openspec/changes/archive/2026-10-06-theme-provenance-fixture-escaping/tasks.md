# Tasks

## 1. Expected display provenance

- [x] 1.1 Correct only the expected local source in `packages/kuru-core/tests/config_schema.rs` to escape and bound the supplied path under the existing display contract; preserve every schema, layering, managed and authority assertion.
- [x] 1.2 Run the owning exact theme contract case and relevant host/Windows lint, typecheck, format and managed checks; record actual passes and native Windows CI limits.
- [x] 1.3 Settle final strict/apply readiness, read every returned context and record truthful acceptance before the normal archive/commit/fresh-CI workflow.

## Observed evidence

The original native Windows ARM and x64 partition4 jobs 112572830804 and 112572294249 fail the same raw-versus-escaped source assertion at line142; their official logs preserve both strings. The implementation still uses the supplied path and `char::escape_default` followed by the existing 160-character bound, without canonicalization. Root reviewed the exact fixture-only diff clear; no other assertion or production code changed.

The owning core task23807 selected the exact theme contract and passed1/1 in0.05s; all other test targets filtered0. Host lint71992, Windows-target lint6399 using standard `CARGO_BUILD_TARGET` with the existing core task, all-target/typecheck87069, format60063 and managed check all exited0. Initial sandbox MBX/format invocations stopped before compilation; the nonexistent core `lint:windows` address was a task-resolution error. Corrected existing-task invocations above passed; those initial errors are not behavior passes or failures.

Native Windows execution, complete hosted coverage/90%, final-head PR checks and exact merged-main acceptance remain pending fresh CI. No unchanged workflow retry, paid run, production rendering change, deadline or policy relaxation is part of this correction.

Final strict validation and apply gate exited0, and all three returned context files were read in full. The final readiness task describes this observed gate settlement; physical archive, normal commit/push and fresh CI follow through their owning workflow rather than claiming those future outcomes here.
