## 1. Failure-only native diagnostics

- [x] 1.1 Add terminal bundle status context containing only target, attempt out of three, status, and `Retry-After` presence, with focused assertions and no retry-policy change
  Terminal 503 and permanent/`Retry-After` fixtures now assert target,
  attempt/three, status, and presence without adding a URL or header value.
- [x] 1.2 Trace only the final missing-archive bootstrap fixture using isolated Bash 3.2-compatible child state, preserving its original assertion, 30-second deadline, bounded failure-only stderr, and every other bootstrap case
  The exact final fixture keeps its existing command and deadline, adds isolated
  `SHELLOPTS=xtrace`/`PS4`, and asserts bounded root, line, producer, consumer,
  and wait milestones only after the expected failure.
- [x] 1.3 Pass the focused bundle recovery and missing-archive bootstrap cases, delivery formatting, all-target/all-feature typecheck, strict Cospec validation, and diff checks
  Bundle recovery passed 8/8 in 7.70 seconds; the exact bootstrap case passed
  1/1 in 0.33 seconds; delivery typecheck, repository format check, strict
  validation, and diff checks passed. A fixture-equivalent Bash trace was
  12,264 bytes, below the existing 16 KiB diagnostic bound.
- [x] 1.4 Record the inconclusive native failures and preserve post-push required GitHub checks as the merge gate without claiming a root cause or behavior correction
  Intel job 104013546597 ended on a terminal pinned-archive 504 whose attempt
  and `Retry-After` branch were not visible. The ARM bootstrap failure did not
  reproduce in 25/25 executions of the existing instrumented binary. Required
  post-push checks remain the merge gate; neither cause is claimed.
