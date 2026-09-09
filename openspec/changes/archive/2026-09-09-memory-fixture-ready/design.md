## Context

Libtest's single-thread reporter writes `test NAME ... ` without a newline before the child prints readiness. Exact whole-line matching therefore blocks even though the child reached its start barrier.

## Decisions

Use a unique final whitespace-delimited token and preserve the existing stdin start barrier. Force only the child harness to one thread; each child executes exactly one test already, and all four processes remain concurrent. Read stdout in a thread, retain diagnostics, and wait at most five seconds for readiness. On timeout or early exit, kill and reap this fixture's children before reporting captured output.

## Risks / Trade-offs

The token is emitted only by this fixture and carries no user input. Run the actual process fixture with RUST_TEST_THREADS=1 under an external deadline, then confirm its normal invocation also completes and preserves all four messages.

## Integration contract

Readiness is a distinct token emitted after libtest's optional prefix. Existing child status checks and durable-message assertions remain mandatory; parsing readiness does not substitute for successful completion.
