# Tasks

## 1. Ordinary main termination

- [x] 1.1 Return standard ExitCode after settled dispatch, preserving typed statuses/diagnostics and untyped errors; document ordinary exit/profile collection.
- [x] 1.2 Run existing CLI/doctor command, interruption and backpressure checks plus host/Windows lint and independent review.
- [x] 1.3 Verify the native Windows regression: previously missing doctor error branches executed by the same existing cases emit counters after correction; compare with saved76a076ea235/461 and retain all status/privacy assertions.

Observed native regression: exact861811ef CI37872687339 Windows partition3 passes the unchanged invocation_failure_uses_fixed_redacted_report case and publishes its valid export. The invocation_failure group at257:1 is14/14 versus saved76a076ea0/14 (whose full doctor total was235/461); DoctorExit::fmt234–236 is3/3 versus0/3. Doctor implementation and test files are unchanged. Complete workspace coverage is assessed separately by its canonical merge; no partial workspace metric is claimed.
