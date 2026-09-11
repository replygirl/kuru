## 1. Observe the existing Windows shell contract

- [x] 1.1 Add distinct bounded private stage markers to the negative control and Kuru shell flow in apps/kuru-tui/tests/windows_cli.rs, retaining all hash, environment, identity and deadline assertions. Source review and git diff --check passed; native stage observations remain pending.
- [x] 1.2 Run native Windows acceptance and record the actual stage sequence or failing stage. CI 34607501091, Windows job 103289452218, reached entered/version-checked/hash-started, then timed out after 30 seconds with the root still running. No hash completion marker appeared; command discovery and the hash body remain distinct hypotheses. Log: /tmp/kuru-windows-resume-ci-job-103289452218.log.
- [ ] 1.3 Add bounded fixed-label pre/post command lookup observers and one timeout-only separate Core debugger trace, preserving the original hash expression, failure and 30-second budget; verify actual native lookup/trace evidence.
- [ ] 1.4 Validate and archive the completed fixture diagnostics before the final PR commit, preserving the distinction between diagnostic evidence and a passing shell acceptance test.
