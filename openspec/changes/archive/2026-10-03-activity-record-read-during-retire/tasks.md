# Tasks

## 1. Diagnose

- [x] 1.1 Read the job log, the publisher's write/replace path, the platform replace and the client reader, and verify the ordering on every OS; record measured vs inferred in the roadmap handoff
- [x] 1.2 Make every activity test that reduced a record read or lookup to a boolean report the error, and verify the activity tests pass

## 2. Fix

- [x] 2.1 Join the publisher once, keeping its last activity, and use it from `mark_failing` and `retire`; verify that the marks-then-retires tests pass
- [x] 2.2 Await the publisher's last write in the close between the listener drop and the endpoint retire; verify the regression test

## 3. Regression and docs

- [x] 3.1 Add `the_publishers_last_write_lands_before_the_endpoint_retires` and verify it fails without 2.2 and passes with it
- [x] 3.2 Document the close order of the record in `docs/development.md` and verify `docs:check`
- [x] 3.3 Run the memory tests, format, lint, Windows lint, typecheck, docs and cospec validation, and record the observed results in verification
