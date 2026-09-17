## 1. Bounded post-validation observation [critical]

- [~] 1.1 @regression (agent) run the existing native Windows packaged install/update acceptance through a timeout after release validation -> defer: hosted Windows is required to observe the new timeout markers and real post-cleanup inventory.
- [x] 1.2 @integration (agent) exercise the fixture-owned metadata formatter against absent stage, present private stage, regular candidate/target, and unsupported entry types -> `mise run //apps/kuru-tui:test:embedded-runtime` passed 3/3 on 2026-09-17; two focused formatter tests established bounded absent/regular/stage/candidate handling and no install-symlink traversal.

## 2. Preserved installer behavior

- [x] 2.1 @integration (agent) run the packaged embedded-runtime acceptance on the supported host -> the same `mise run //apps/kuru-tui:test:embedded-runtime` completed in 42.43 seconds; packaged direct install and update persisted complete offline memory while retaining the existing 100-second budget and success phase assertion.
- [~] 2.2 @e2e (agent) run the native Windows application and installation checks -> defer: hosted Windows is required to exercise PowerShell bootstrap and a real timeout inventory.

## 3. Static ownership review

- [x] 3.1 @eval (agent) review the final diff -> Franklin independently cleared the diff on 2026-09-17: markers only after stage construction/before `Native.Publish`; error-only 64-entry/4 KiB direct metadata; no contents, symlink traversal, deadline, retry, bridge, process-wrapper, or success-path behavior change. App typecheck and lint passed in 1.06 and 5.96 seconds; `mise run format:code` and `git diff --check` passed.
