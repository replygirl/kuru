# Verification

## 1. Startup diagnostic evidence [critical]

- [x] 1.1 @regression (agent) inspect the failed ARM case and both fixture owner opens before and after the correction -> prior CI 37665200229/job 112942954144 at d57f21ea reported exit1 without the log; reviewed diff now opts both opens into the existing capture before temporary-root teardown, labels initial/successor phases and adds reopen iteration context. This proves the diagnostic gap was corrected, not the original Dolt failure cured.
- [x] 1.2 @unit (agent) run startup_log_capture_is_opt_in_exact_and_bounded -> owning memory task 60094 EXIT0 selected the exact existing case, which passed; bounded active/staged capture, unrelated-error preservation and ambiguous/unsafe refusal assertions are unchanged.
- [x] 1.3 @integration (agent) run checked_recovery_ends_on_the_verified_successor -> owning memory task 60094 EXIT0 selected the exact case, which passed with both original iterations and assertions; combined selection 2/2, 0 ignored, 9.19s body, 168.92s total including prepared bundle/supervisor dependencies. Local macOS proof only; original ARM exit1 remains unexplained. Afterwards only successor error-context wording changed from verified successor to successor, since verification occurs later; no repeated native suite for wording alone.

- [x] 1.4 @equivalence (agent) run Rust formatting -> owning format:rust tasks 77109 and final 51225 both EXIT0, including the final wording refinement.

## 2. Delivery limits

- [~] 2.1 @runtime (agent) establish the original ARM Dolt early-exit cause -> defer: retained artifact has no server.log; local success cannot establish historical cause or cure. Fresh full CI must accept the new head and expose bounded diagnostics on any recurrence.
- [~] 2.2 @equivalence (agent) run broad host/Windows statics -> defer: Root owns the integrating normal push hooks; no duplicated broad static suite for these existing helper calls.
