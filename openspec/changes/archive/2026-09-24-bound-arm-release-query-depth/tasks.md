## 1. Bound the observed compiler query

- [x] 1.1 Keep the pre-fix Ubuntu ARM release job's exact query-depth failure as the regression baseline and add the finite 256 limit at the `kuru` library crate root.
- [x] 1.2 Record the same release build's corrected exact-head Ubuntu ARM pass as a hard pre-merge regression gate. It remains deferred in `verification.md` until the corrected branch is pushed.

## 2. Review and publish

- [x] 2.1 Run owning TUI release build and source format checks, and obtain bounded independent source review.
- [x] 2.2 Strict-validate and archive the fix record before the final branch checkpoint; keep native ARM and full CI gates separate.
