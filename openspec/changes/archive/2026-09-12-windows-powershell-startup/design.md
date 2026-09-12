## Context

The existing native PowerShell regression uses `take(65537).read_to_end` on each captured stream, joins both readers, and checks their lengths only after both finish. Reading the extra byte completes the bounded reader without reporting an error. An oversized writer can then block on its undrained stream while keeping its other stream open, so the join waits until the overall timeout. This is a concrete capture failure independent of the unconfirmed historical PowerShell stall.

The original timeout log omitted the launch label, partial output, and retained process state. Diagnostic repetitions and 16 fresh Windows runners have not reproduced that stall. The final correction covers observable capture and cleanup behavior without claiming a PowerShell startup diagnosis.

## Goals / Non-Goals

**Goals:** Report an exceeded stream limit promptly, keep useful bounded failure evidence, and complete owned process and pipe cleanup. Prove these paths with real native children.

**Non-Goals:** Change PowerShell startup, add cache or JIT workarounds, alter production process APIs, increase timeouts, retry failed acceptance, change release gating, or retain a diagnostic CI matrix.

## Decisions

Use a bounded asynchronous reader for each stream that returns an error immediately upon reading the byte beyond 64 KiB. Keep at most the limit plus that diagnostic byte per stream. Joining these readers propagates either stream's limit error immediately while preserving the normal concurrent drain.

Capture failure must retain the original error, launch label, and partial stdout/stderr. Inspect a retained root process handle separately from the owned child job's quiescence: a live descendant and a live root are different observations. Collect state before termination, then request owned termination and await process-tree exit and pipe cancellation within bounded cleanup budgets. Include cleanup failures in the diagnostic without replacing the original capture failure.

Keep the released PowerShell command unchanged and perform one direct and one configured launch. Additional progress writes before .NET initialization change execution order and therefore cannot serve as the original-script acceptance path.

Exercise failure handling with deterministic native process fixtures. An oversized writer must continue writing enough data to block if a capped reader stops draining; keep the other stream open so the old join behavior would reach the overall timeout. A stalled fixture must produce known partial output and stay alive until cleanup. Assert the resulting failure category, retained evidence, and actual process exit.

Remove temporary stress repetitions and the 16-runner CI matrix. The ordinary Windows platform coverage job remains the native acceptance environment.

## Risks / Trade-offs

Real process scheduling varies across runners. Synchronize fixtures with observable output and assert error categories and retained process exit; do not substitute a shorter success deadline for the existing PowerShell budget.

A read error cancels the sibling Rust future while native overlapped I/O may remain pending. Await the existing pipe cancellation/close contract before releasing its resources, and retain ownership through cleanup failure.

Better failure diagnostics do not explain the original sporadic PowerShell stall. Preserve that limitation in the verification record and release description.
