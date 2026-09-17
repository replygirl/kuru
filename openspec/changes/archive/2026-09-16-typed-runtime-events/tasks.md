## 1. Semantic event foundation

- [x] 1.1 Add `kuru-runtime` event variants, tool receipt types, projection helpers, and compatible wire adapter; verify enum/wire/projection unit and integration fixtures
- [x] 1.2 Migrate harness broadcast, trace, completed output, engine emission, and runtime tests to projected semantic events; verify fake-secret data cannot reach any public event path

## 2. Settled tool observation

- [x] 2.1 Route cognitive and external tool invocation admission and settlement through one projected receipt builder; verify success, error, denial, budget failure, cancellation, bounds, byte counts, digest, and duration fixtures
- [x] 2.2 Preserve one receipt per call and existing response/checkpoint behavior; verify completed exact retry neither dispatches nor adds a second observation

## 3. Journal and interactive adapters

- [x] 3.1 Implement format-2 completed journal writing and explicit v1/v2 replay normalization; verify persisted v2 reopen, v1 replay, malformed v2 withholding, unknown historical kinds, and possible-dispatch refusal
- [x] 3.2 Update TUI and CLI consumers to pattern-match semantic events while retaining the three-key wire output and final-answer behavior; verify TUI activity fixtures do not render peer bodies

## 4. Documentation and integrated checks

- [x] 4.1 Document compatible wire serialization, projected receipt measurement, and replay behavior; verify docs checks
- [x] 4.2 Run focused package-owned runtime and TUI checks and record observed evidence in `verification.md`; verify every critical behavior at its declared integration or runtime layer
