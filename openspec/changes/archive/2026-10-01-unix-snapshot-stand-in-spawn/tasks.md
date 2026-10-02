# Tasks

## 1. Create the stand-in without a write descriptor here

- [x] 1.1 Create the stalled-`ps` stand-in from a waited `/bin/sh` child, keep the three original bounded assertions, and verify the test passes
- [x] 1.2 Assert after creation that this process's descriptor table holds no descriptor to the stand-in (Linux only; compiled out on macOS, see verification 1.3), and document the fork/exec mechanism in the test
- [x] 1.3 Add the Linux-only control test showing that a descriptor inherited by a live child blocks exec of the file

## 2. Verify and deliver

- [x] 2.1 Run the unix_snapshot binary repeatedly on the host and in a Linux container; reproduce the old race in the container where possible
- [x] 2.2 Run the kuru-platform test task, format check, lint, typecheck, strict Cospec validation and apply, record observed results and name unrun checks
