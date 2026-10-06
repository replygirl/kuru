# Verification

## 1. Failed-export evidence [critical]

- [x] 1.1 @integration (agent) fail the existing orchestrator's export with malformed profile bytes and recorded sibling/run identities -> owning task 80690 exit 0, failed-export fixture passed: existing failure.txt names the simulated actual run PID and sibling module, size/time and two captured bytes; original export error remains and no receipt exists. This uses the existing fake Host, not actual LLVM corruption.
- [x] 1.2 @unit (agent) exercise missing/reused PID attribution and bounded header capture, including capture errors -> 80690 exit 0, three new evidence cases and five existing attribution cases passed; empty/64-byte prefix, unavailable nonregular prefix and existing total-size bound preserve the primary failure and qualify missing/reused identities.
- [~] 1.3 @runtime (agent) execute native Windows x64 CI from the completed branch and exact main -> defer: remote delivery follows archive/commit; the next actual failure or pass supplies evidence. This patch does not prove the recurring corruption fixed or identify the historical writer.

## 2. Required local checks

- [x] 2.1 @integration (agent) run owning delivery host/Windows lint, all-target typecheck, docs, format and managed/strict checks -> typecheck 63777 and host/Windows lint, docs, format and managed task 48264 exit 0; after the reviewed recording-error path correction, host lint/typecheck 45961 passed and corrected Windows lint/format task 20591 exit 0. Its sole affected real-libtest rerun passed 1/1 in 0.34 s. No broad coverage/native suite or paid request was run.

## Observed scope and limits

The initial focused invocation stopped at Cargo argument parsing before tests;
80690 corrected only the separator and passed 10 selected tests in 0.74 s.
All other delivery targets selected zero tests. The existing real libtest case
ran on the host; its actual PID/recording-error assertions are Windows-only and
remain native-CI pending. The recording-error source correction retains the same
Job/EOF supervision before reporting failed diagnostics. Windows lint 45961
found a test-only `unwrap_err` Debug bound, fixed with `err().expect` without
adding a production trait or changing execution policy.

Independent final source review cleared the captured recording-error ordering,
same-child supervision, qualified prefixes and unchanged strict policy. Final
Cospec strict/apply and archive follow this local evidence; the completed branch
is handed to the sole remote delivery owner for actual CI.

The official 287/75 successful test ledger plus the two corrupt names establish
repeated strict-export failure, not a writer identity. Listing-process PIDs were
recorded; actual run PIDs and sibling profile inventory were absent from those
artifacts. This correction adds those observations and partial prefixes, not a
claim that the historical files were empty, truncated, or written by the CLI.
Prefix capture can race live writers; unchanged metadata does not prove immutable
bytes. Later-section corruption and unrecorded children may need further targeted
evidence. No existing profile is rewritten, repaired or filtered.
