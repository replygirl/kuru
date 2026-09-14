## 1. Bounded download recovery

- [x] 1.1 Extend the existing three-attempt downloader to retry only transient connect/send-timeout and body-chunk network failures within the unchanged 120-second total budget, resetting the private stage and digest before every retry
- [x] 1.2 Add loopback regression fixtures that fail before the correction and prove interrupted-body recovery, persistent bounded failure, exact staging reset, and unchanged permanent/integrity/cancellation behavior
- [x] 1.3 Update `docs/development.md` to describe transient transport recovery, per-attempt connect/read-idle bounds, and the unchanged total, integrity, and publication limits

## 2. Verification

- [x] 2.1 Pass delivery package formatting, focused bundle recovery tests, all-target/all-feature typecheck, Clippy with warnings denied, strict Cospec validation, and diff checks
- [x] 2.2 Record the pre-fix hosted Ubuntu setup failure and preserve post-push required GitHub checks as the merge gate, separately from the independent Windows empty-profile correction
