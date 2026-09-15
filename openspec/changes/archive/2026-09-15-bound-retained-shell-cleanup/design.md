## Context

An `OwnedProcessGroup` must remain anchored until the platform confirms group absence; relinquishing it to satisfy an accounting limit would permit an unsafe stale-PID signal. The current registry tracks ownership only per host, and its retained retry repeatedly invokes a short, polling cleanup window.

## Goals / Non-Goals

**Goals:**

- Bound accepted Unix shell workers and retained groups across every `ShellRegistry` in the process.
- Bound the rate of actual phase-safe platform observations after the caller's cleanup allowance expires.
- Preserve cancellation, primary errors, and owned cleanup until confirmation.

**Non-Goals:**

- Replace per-owner workers with a daemon/reaper framework.
- Add a cleanup give-up deadline, numeric-PID signaling, queues, configuration, or environment controls.

## Decisions

- Use a module-private shared admission pool with a fixed cap. A permit moves into `WorkerFinish`, so it survives registry and caller drops and releases only on a prelaunch completion or confirmed cleanup.
- Keep finite cleanup's existing short poll to retain its prompt bounded-operation behavior. Retained cleanup instead performs one existing phase-safe cleanup attempt per retry and sleeps with a capped exponential interval after failures.
- Treat every uncertain, interrupted, or panic observation as a retained retry. Only existing confirmed absence releases the permit; a phase error remains truthful ownership failure rather than an excuse to drop the owner.

## Risks / Trade-offs

- [A retained owner can occupy a slot indefinitely] → That is required to retain safe ownership; immediate fixed rejection bounds further growth.
- [Backoff delays eventual confirmation] → The initial retry remains short and intervals cap, while eliminating the prior sustained 100 Hz observation loop.
