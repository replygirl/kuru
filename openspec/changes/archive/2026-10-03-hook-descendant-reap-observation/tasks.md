# Tasks

## 1. Diagnose

- [x] 1.1 Read the hook host's cancellation and reap path and the platform's group termination and post-reap observation, and verify a backgrounded subshell of `/bin/sh -c` shares the root's process group by listing the group of a `setpgrp` launched shell
- [x] 1.2 Try to reproduce the timing failure with the unchanged test under CPU load and record the result

## 2. Event-driven assertion

- [x] 2.1 Have the hook's long-lived backgrounded subshell publish the group id to the existing marker path, and after the dream returns require no member of ours in the group; remove the 1200 ms sleep and the survived-file assertion, keeping the `survived` binding as failure evidence; change only the script string and the tail so PR #181's rewrite of the middle of the test merges cleanly; verify the test passes
- [x] 2.2 Regression: with the platform's group signal disabled, verify the new test fails on the group observation listing the surviving descendant; restore and verify it passes
- [x] 2.3 Trial-merge PR #181 (`da4033ed`) into the fix commit and trial-rebase #181 onto it; record conflicts, compile and the merged test result

## 3. Checks

- [x] 3.1 Run the checks in verification.md and record the evidence
