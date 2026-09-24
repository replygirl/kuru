## Why

An exact selected candidate-abandon command failed on Windows with a generic storage error while the ref remained open. The owner stage and Dolt error were hidden, and a focused real-Dolt test did not reproduce the suspected post-close session race, so a production correction would currently be guesswork.

## What Changes

- `packages/kuru-memory/src/store/operational_gc_tests.rs` records bounded real-Dolt evidence for exact candidate-pool session retirement and checked branch transition.
- Test-support code in `packages/kuru-memory` launches an owned service with a private diagnostic file and emits only fixed candidate stage plus validated SQL error classification on failure.
- `apps/kuru-tui/tests/cli.rs` reads only those bounded allowlisted records when the existing selected-abandon CLI assertion fails; its success and exact-ref assertions remain unchanged.

## Impact

Only native fixtures and test-support behavior change. The public RPC, normal service output, product mutation path, and 90% coverage threshold remain unchanged; one focused real-Dolt fixture adds bounded test time.
