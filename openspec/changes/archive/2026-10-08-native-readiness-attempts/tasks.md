# Tasks

## 1. Native observation assertions

- [x] 1.1 Correct `packages/kuru-platform/src/unix.rs` to retain zero running-root jobs, count observed refused attempts separately from successful readiness, require exactly one fresh job after consuming readiness, and complete native cleanup before assertions; verify the focused test and existing spawn-lock contention contract on macOS. Observed: both focused native tests pass (1/1 each), with unchanged deadline and polling cadence.
- [x] 1.2 Verify same-class owner-local counter assertions, format and platform lint, and independently review the native contract; record CI37853387922's actual original failure without claiming its first refusal classification was captured. Observed: format and package lint pass; independent source review clears the correction and finds no other total-attempt/success assumptions. Original job113571954350 reports total attempts2 vs expected1; its first classification was not recorded.
