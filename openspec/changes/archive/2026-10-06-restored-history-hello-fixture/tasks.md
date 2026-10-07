# Tasks

## 1. Exact hello expectation

- [x] 1.1 Add only `history_scope:null` to the existing no-starter-token exact JSON expectation and verify all other assertions and production bytes remain unchanged.
- [x] 1.2 Run the owning starter-token and pinned protocol-surface cases, affected host/Windows lint, typecheck, formatting and managed checks; record actual results and unrun native limits.
- [x] 1.3 Complete strict validation/apply context review and archive readiness with truthful evidence before normal final commit and fresh full PR CI.

## Acceptance

The exact starter-token test must match the declared optional history field while
retaining parser rejection, token omission/privacy and authority verification.
The existing pinned protocol-surface test must pass without changing production
serialization or its declared 1.13 fixture. Native Windows/Linux execution,
coverage and exact-main acceptance remain fresh hosted CI requirements.

## Evidence

Published `1ad442f7` Ubuntu coverage partition 4 failed the existing starter-token
fixture with `a hello without a starter token changed its wire form` (109 memory
library cases passed, one failed, one ignored). Source review confirms the added
optional history field serializes null; the old expected string omitted it.
No retry or production correction is claimed.

Observed local acceptance: the owning memory task passed both exact existing
starter-token and pinned protocol-surface cases (2/2, no ignored, 0.01s; all other
targets filtered to zero). Host lint, Windows-target lint, all-target/all-feature
typecheck, Rust/TOML formatting and managed checks exited 0. The initial strict
and apply gate exited 0 and all three returned context files were read before
editing. Source review confirmed the single expected-JSON insertion; production
and all other assertions are byte-preserved. Native Linux/Windows execution,
workspace coverage, fresh complete PR CI and exact-main acceptance are unrun at
this local checkpoint and remain required hosted checks. Original Ubuntu and
Linux ARM failures have the same omitted-null-field error; neither was retried.
