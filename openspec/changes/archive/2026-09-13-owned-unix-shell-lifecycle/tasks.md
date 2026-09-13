## 1. Safe Unix platform owner

- [x] 1.1 Add the narrow platform-owned standard-child/fresh-group state machine with bounded `EINTR`, permanent ownership-loss disarming, group-then-root signal consumption, exact status reap, and post-reap absence observation.
- [x] 1.2 Add deterministic platform transition tests and real native process-group fixtures that prove no child/PID escape, no post-reap signal, root-movement handling, exact status, and `EPERM`-to-`ESRCH` semantics.

## 2. Connector shell owner

- [x] 2.1 Replace the Unix built-in shell's numeric Drop guard with a private registered OS-thread owner that retains the platform child, converted bounded pipes, projected environment, primary result, and exact workspace `Directory`.
- [x] 2.2 Start the operation deadline at acceptance; implement fair EOF/root observation, one five-second cleanup allowance, fixed primary-plus-cleanup diagnostics, and retained delayed confirmation after the caller bound.
- [x] 2.3 Close shell registration and cancel/await starting, active, and unconfirmed owners in `ToolHost::shutdown` while preserving MCP cleanup and the existing Harness/MemoryStore boundary.

## 3. Regression and lifecycle fixtures

- [x] 3.1 Record the inherited-pipe pre-fix source/ordering gap without manufacturing a timing failure, then add a deterministic transition regression and real post-fix native fixtures for timeout, natural root, pipes-before-root, silent descendant, stdout/stderr overflow, read failure, cancellation, and parent-runtime loss.
- [x] 3.2 Add controlled startup/shutdown/panic fixtures proving a caller fallback at operation deadline plus one cleanup allowance, no late spawn, exact reservation reclamation, registration closure, and no multiplied cleanup windows.
- [x] 3.3 Add delayed-confirmation evidence proving an unconfirmed worker remains registered past caller return, can recover from bounded `EINTR` to perform its one initial destructive transition, then reaps without another signal and self-removes during a long-lived host.

## 4. Compatibility, documentation, and verification

- [x] 4.1 Preserve and run existing shell environment, status/output/input/timeout, workspace-root, stdio MCP, Windows Job, and actor-to-tool behavior; add the fake-provider conversational CLI fixture using delivery's existing bounded runner through a TUI-only dev feature, and run native macOS and Linux lifecycle fixtures through owning package tasks.
- [x] 4.2 Update `AGENTS.md`, platform crate/module descriptions, `docs/protocols.md`, and the curated tools reference with the bounded Unix owner and its limitations, without a general process framework, sandbox, escaped-process, memory-lease, or synchronous-Drop claim.
- [x] 4.3 Run focused platform/connectors format, lint, typecheck, and package tests plus the single coordinated workspace coverage task; record native results and leave unavailable platform evidence explicitly unchecked.
- [x] 4.4 Keep built-in shell dispatch target-specific so Unix passes its owned registry while Windows retains its existing root-capability contract; verify both native targets compile and the Windows shell fixture still runs through the platform-owned process boundary.
