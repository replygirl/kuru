Current acceptance is complete at a1547ed; see final integration below and
native-windows verification, "Final native acceptance: a1547ed". Historical
upstream failures and controlled RED/GREEN evidence retain their original meaning.

## 1. Immutable HTTP recovery [critical]

- [x] 1.1 @regression (agent) Serve actual HTTP 500 followed by the pinned body through a local TCP server and call the real preparation path -> the unchanged downloader failed on the real 500 response; after repair the same test passed, requiring two identical GETs, exact verified bytes and the digest-named published file. RED/GREEN logs are `/tmp/kuru-bundle-retry-red.log` and `/tmp/kuru-bundle-retry-green.log`.
- [x] 1.2 @integration (agent) Serve transient response sequences, permanent errors, Retry-After responses and corrupt successful bodies -> all nine focused host tests passed in 6.42 seconds. Each eligible status recovered on attempt three; persistent 503 stopped at three; permanent/advice errors, malformed headers, truncated/oversized/incorrect bytes failed after one request with clean staging and unchanged stable-lock identity. Native integration remains in row 2.2.
- [x] 1.3 @integration (agent) Stall a subsequent attempt or cancel after a real transient response -> the actual host fixtures observed shared test budgets of one and two seconds across backoffs/headers/body, earlier than the ten-second request timer; an observed error-socket close preceded cancellation, which removed staging/released the same lock and left no reconnect during 600 ms. These are cooperative async bounds, not filesystem preemption claims.

## 2. Owning integration

- [x] 2.1 @integration (agent) Run the existing delivery bundle regressions plus relevant strict lint/format/cospec/docs checks -> import, HTTP, cancellation, private identity and publication behavior remain valid with exact dependency pins.
- [x] 2.2 @runtime (agent) Run normal concurrent hooks and observe the subsequent automatic native CI -> all required gates pass with measured coverage; record the observed upstream HTTP 500 separately from deterministic recovery fixtures and preserve release/Pages scheduling.
- [x] 2.3 @manual (agent) Review build-preparation command/error behavior and owning developer guidance against the implementation and observed fixtures -> independent review confirmed retry scope, fixed pins/async deadline, original integrity errors, local import/offline instructions and unchanged success output; clarified that only error responses with Retry-After remain errors. Report: `/tmp/kuru-bundle-retry-independent-review.md`.

The observed pre-fix upstream failure is CI 34572200638 Ubuntu job 103176534485,
at 07:01:16Z: HTTP 500 for the exact official Dolt 2.3.3 Windows ZIP. Preparation
failed before instrumentation; no Ubuntu test/coverage result exists for that
job. Its raw log is `/tmp/kuru-ci-563d321-ubuntu.log`. A live GitHub failure
does not establish deterministic recovery; the isolated response-sequence
regression above is required.

## Final integration: a1547ed

[CI 34589999794](https://github.com/replygirl/kuru/actions/runs/34589999794)
passed the complete required graph; aggregate 103240993431 succeeded at
11:08:31Z on September 11. Actual final source equivalence, native counts,
raw coverage and shipping phases are recorded in native-windows verification,
"Final native acceptance: a1547ed". All full native coverage gates exceeded 90%,
and all seven static categories, dependency-lock checks and eight normal local
hooks passed with the existing pins.

The full Unix and Windows suites retained the actual preparation regressions:
transient-status recovery, permanent/Retry-After refusal, body/integrity failures,
one total deadline and cancellation, alongside existing local import, cache,
identity, publication and cleanup controls. The original 500-to-200 RED and nine
focused GREEN cases above are preserved. Current successful upstream preparation
is not described as an observed live retry.

The policy remains at most three attempts, only for the selected transient
statuses without Retry-After, with 250 ms / 1 s delays inside the existing
120-second total budget. Integrity and private-object checks, ordinary protocol
retry behavior and Release/Pages scheduling are unchanged. Existing independent
manual review remains applicable to the final source and guidance. Evidence:
`/tmp/kuru-ci-a1547ed-results.md`, `/tmp/kuru-windows-a1547ed-results.md`, their
completed control/count logs, and `/tmp/kuru-a1547ed-push.log`.
