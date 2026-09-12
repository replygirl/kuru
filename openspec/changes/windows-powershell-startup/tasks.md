## 1. Correct native capture failure handling

- [ ] 1.1 Replace post-join stream-size assertions with bounded readers that return an output-limit error immediately after either stream exceeds 64 KiB while preserving bounded partial output.
- [ ] 1.2 Preserve launch identity and separate retained root-process state from owned job quiescence on capture failure; await owned termination, process-tree exit, and pipe cleanup before reporting the original error.

## 2. Prove the failure contracts

- [ ] 2.1 Add a real oversized native writer regression that fails before the correction by reaching the overall timeout and passes afterward by reporting the output-limit error promptly and reaping the owned process.
- [ ] 2.2 Add a real stalled-child control that reaches its capture timeout with known partial output, preserves that evidence, and proves owned process cleanup.
- [ ] 2.3 Verify the unchanged stock PowerShell 5.1 script once through each direct and configured launch, including actual .NET initialization, exact output, script marker, and quiescent process tree within the existing 30-second capture budget.

## 3. Restore and verify ordinary delivery

- [ ] 3.1 Remove diagnostic stress repetitions and the temporary native CI matrix, restoring the normal Windows platform job and coverage artifact name.
- [ ] 3.2 Run the affected format, Windows-target lint/type checks, native coverage with the 90% gate, and strict cospec validation; record actual results before archive.
