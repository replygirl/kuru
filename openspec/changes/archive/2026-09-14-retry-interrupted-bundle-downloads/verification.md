## 1. Interrupted pinned-archive recovery [critical]

- [x] 1.1 @regression (agent) serve an accepted pinned archive whose first response body is interrupted and whose second response is complete -> hosted run 34849384516 failed the prior source on its first body timeout before Ubuntu tests; the corrected `timed_out_body_frame_retries_with_a_clean_stage` fixture made two identical GETs, discarded the first partial bytes and digest, and published only the complete verified archive
- [x] 1.2 @integration (agent) exercise persistent send/body interruption within an injected short overall budget -> `interrupted_body_retries_only_three_times_and_releases_the_lock` combined a 503 with two interrupted bodies, observed exactly three GETs, no staged output, and the same acquirable stable lock; the existing shared-deadline cases also passed

## 2. Fail-closed boundaries

- [x] 2.1 @equivalence (agent) exercise permanent statuses, `Retry-After`, size/digest corruption, local output failure, and cancellation fixtures -> all eight `bundle::recovery_tests` passed in 7.71 seconds, including one-GET permanent/integrity rejection, bounded cancellation, stage cleanup, and stable lock identity; delivery formatting, all-target/all-feature typecheck, Clippy with warnings denied, strict Cospec validation, and diff checks passed
- [~] 2.2 @runtime (agent) run the exact corrected pull-request head through required CI -> defer: required GitHub checks run only after the completed fix record is archived and pushed, remain the merge gate, and must attribute the independent Windows coverage result separately
