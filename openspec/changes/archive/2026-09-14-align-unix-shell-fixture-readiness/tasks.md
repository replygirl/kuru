## 1. Unix shell fixture readiness

- [x] 1.1 In `apps/kuru-tui/tests/unix_shell_turn.rs`, bound the diagnostic-write fixture's fake-provider readiness observation by the existing CLI operation budget while leaving `CLEANUP_TIMEOUT` and all production, cleanup, ownership, output, and diagnostic assertions unchanged
- [x] 1.2 Add focused deterministic evidence that a held startup can cross the shorter cleanup window and still reach the provider gate within the CLI budget, then pass the exact diagnostic-write test, full application task, strict Cospec validation, and diff checks; leave final formatting and typecheck repetition to the normal pre-push hooks

## Evidence

- The original pre-push run failed at the old five-second `gate.started` observation with `timed out waiting for fake provider gate`; the exact startup cause was not established.
- With a temporary deterministic 5.05-second pre-gate hold, the corrected exact fixture passed in 22.27 seconds. The hold was removed and the intended source restored before review.
- The restored exact application task passed 1/1 in 3.79 seconds. A subsequent selector-placement mistake ran the full application target set to terminal exit 0; its clipped output is not claimed as a separate isolated five-test receipt.
- The final source is limited to the readiness helper and observation diagnostics, has SHA-256 `a62a3d1e96e999d58cb76594563e2983536431878c2c551eacc68c304857acfe`, passed `git diff --check`, and received independent exact-byte review clearance.
- Normal pre-push hooks remain responsible for final repository formatting, typecheck, and full coverage before publication.
