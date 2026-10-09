# Tasks

## 1. Ordinary main termination

- [x] 1.1 Return standard ExitCode after settled dispatch, preserving typed statuses/diagnostics and untyped errors; document ordinary exit/profile collection.
- [x] 1.2 Run existing CLI/doctor command, interruption and backpressure checks plus host/Windows lint and independent review.
- [ ] 1.3 Verify the native Windows regression: previously missing doctor error branches executed by the same existing cases emit counters after correction; compare with saved76a076ea235/461 and retain all status/privacy assertions.
