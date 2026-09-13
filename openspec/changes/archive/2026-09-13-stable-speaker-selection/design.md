## Context

The current selector shifts the traversal of tied peers by session turn count.
No activation change is needed for the voice to change. The correction belongs
in that selector and the existing completion-state publication path.

## Decisions

Use one optional last-completed-speaker field on the existing session record,
defaulting absent when reading older records. Carry it through the existing
persist/reconcile mechanism; do not introduce a SQL table or migration. Automatic
selection orders by activation, previous-speaker preference on a tie, then
ascending identity. Existing target and focus paths retain precedence.

Emit a separate selection-reason event using the current Event shape. The
existing speaker event still distinguishes part versus relationship. Reasons
use fixed text and identity metadata, never private deliberation content.

## Risks / Trade-offs

Continuity favors a prior speaker only among equal maximum activations. An
unavailable or retired identity cannot be revived by stored continuity. A
relationship previously chosen by focus is not an automatic candidate unless
already eligible through the existing path. This is a small built-in correction;
Phase 1 can extract the selector behind its mode contract without a new policy
framework being introduced here.

Completion state follows existing uncertain-publication reconciliation. An
abort after a completed revision has committed must not falsely promise that
the revision did not occur; failure before publication leaves continuity alone.

## Operational surface

The existing native CLI and TUI expose the selected identity and event trace.
There is no new command, prompt, endpoint, secret, process, binary dependency,
deployment topology or configuration option. Native runtime tests cover the
existing supported platforms and real embedded Dolt session persistence.
