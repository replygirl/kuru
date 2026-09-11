## 1. Observe the existing Windows shell contract

- [x] 1.1 Add distinct bounded private stage markers to the negative control and Kuru shell flow in apps/kuru-tui/tests/windows_cli.rs, retaining all hash, environment, identity and deadline assertions. Source review and git diff --check passed; native stage observations remain pending.
- [ ] 1.2 Run native Windows acceptance and record the actual stage sequence or failing stage, then validate and archive the test-only change.
