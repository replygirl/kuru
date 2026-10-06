# Design

## Context

PageUp intentionally selects a stable Reading anchor. Later local command
results append to the transcript while preserving that anchor; the fixture
must explicitly return to follow-tail to inspect those new results.

## Decisions

Use existing PageDown input and completed-frame observations until the visible
history-return marker clears. Keep the existing navigation deadline and all
configuration assertions; change no production behavior or capability spec.

## Risks / Trade-offs

A changed frame alone does not prove tail navigation. Observe the completed
frame and explicit absence of the Reading marker before subsequent commands.

## Operational surface

The existing 120-column, 28-row native PTY fixture runs the owning app's test
binary on the host runner with isolated fake project/data and the demo provider.
It uses existing prepared real Dolt memory and owned terminal cleanup, no
external bind address, provider secrets, additional connections or new binary
versions. Native Ubuntu/macOS CI supplies platform coverage; local macOS checks
exercise this same existing fixture. Windows compilation remains unchanged.
