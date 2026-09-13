## Context

The app already retains a writable `MemoryStore` through CLI execution, and the
TUI loop has a completed-draw boundary before it admits input. Existing memory
state transactions supply the needed durable version and reconciliation without
a schema or runtime service.

## Goals / Non-Goals

**Goals:** present one short useful notice at a truthful visible boundary and
persist only that it was shown for this project/version.

**Non-Goals:** consent, acknowledgement, opt-out, automatic retention, a new
runtime API, provider input, or notice delivery for inspection-only commands.

## Decisions

- A private app `MemoryNotice` owns a store clone, state key, and plain text.
  A runtime notice service was rejected because it would couple memory/UI policy
  to provider and peer execution.
- Headless commands flush stderr before `put`; the TUI records after a completed
  draw. Recording first was rejected because a crash or output failure could
  suppress an unseen notice.
- The public `ui::run` remains unchanged; private `run_with_notice` receives an
  optional adapter. Putting notice state in `View` was rejected because it would
  make rendering tests and model-facing presentation own persistence.

## Risks / Trade-offs

- [Repeated notice after output/write interruption] → prefer a repeat over
  claiming a notice was seen; existing receipt reconciliation prevents duplicate
  committed state where the write completed.
- [Directory text could be interpreted as a command] → use the existing escaped
  display representation and describe controls as literal commands only.

## Operational surface

The notice runs only inside the existing local CLI process after writable memory
opens. It creates no listener, container service, credential, network connection,
binary selection, or platform-specific runtime dependency. Existing native CLI
and TUI fixtures remain the supported execution surface.
