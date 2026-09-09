## Context

Main run 34378748501 failed in tui_smoke.py at child.wait(timeout=5).
The fixture queues actions on 150 ms intervals, while Enter during a busy
operation preserves the draft without submitting. It also stops draining the
terminal before exit, so a final redraw can block on a full PTY buffer.

## Decisions

Use a shared terminal fixture driver that reconstructs cursor-addressed screen
updates, waits on explicit expected states, and drains output until process exit.
Retain fixed observation windows only when testing elapsed-time animation.
Introduce controlled process scheduling pauses in the real smoke flow and a
backpressure regression for the driver. Keep failures bounded and diagnostic.

## Risks / Trade-offs

Screen assertions must replay cursor-addressed diffs, because stripping ANSI
can miss unchanged text or match stale text. Keep the supported escape subset
limited to what the existing Ratatui backend emits, and reuse it consistently.

## Operational surface

Existing macOS and Ubuntu GitHub runners and local POSIX PTYs. No new tools,
secrets, binds, containers or platform support. CI retains the complete check
gate, source installation smoke and 90% workspace line coverage threshold.
