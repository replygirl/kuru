# Tasks

## 1. Settle the intercepted session before waking the client

- [x] 1.1 Retain client and upstream streams with borrowed forwarding halves;
      close upstream, observe session end, then release the client at an interception.
- [x] 1.2 Include the actual proxy task error in shutdown diagnostics while
      preserving strict failure handling and existing deadlines.
- [x] 1.3 Preserve ordinary half-close reply draining with a real TCP regression;
      run both production lost-commit cases and absent-usage/validation request cases,
      then owning lint and formatting.

## Observed evidence

PR #263 full CI37755037885 passed (62 successes / four expected skips), and all
PR checks passed before merge. Exact-main CI37758767569 job113250111301 failed
the successful production lost-commit case while waiting for the proxy's
session-end flag. The earlier early-error case passed in main's partition 7.
Source establishes that client EOF permits production to retire the migration
pool used by the proxy observer; forwarding `try_join!` can also cancel that
observation. The previous panic omitted the underlying SQL/socket error, so its
exact historical error remains unknown. No main pass or blanket cause cure is
claimed.

The final owning memory task ran all five named cases: 5 passed, 0 failed,
0 ignored. The real TCP regression observed client FIN before the exact upstream
reply and subsequent client EOF, with neither fault flag set. Both real-server
lost-commit cases and both absent-request cases passed. Final Rust formatting and
the package's all-target/all-feature Clippy task passed with warnings denied.
Independent final review found no material issue. Full follow-up PR and exact-main
CI remain pending; no production behavior or deadline was changed.
