## Context

`Harness::run` already returns the text, speaker ID, relationship, input/output
usage and limit state. The TUI discards this value while consuming the runtime's
256-entry broadcast as if it were a reliable completion channel. Completion and
activity can arrive in different orders, including after cancellation.

## Decisions

Use an application-local typed dispatch outcome to distinguish command feedback
from a completed turn. Carry it through the existing generation-tagged completion
channel. The accepted completion appends the returned answer once and updates its
speaker and usage facts. Resolve display names using the result relationship and
known topology, with the returned ID as a stable fallback.

Keep `TurnOutput` and the eight-field `run --json` serialization unchanged. The
new outcome and completion metadata stay inside the TUI application layer.

Activity events may update working indicators and routes, but cannot append answer
text. A delayed speaker event cannot replace the final speaker after completion.
Preserve generation rejection on cancellation and the existing command feedback
path. Do not replay the result's activity trace into the transcript.

## Risks / Trade-offs

Completion is delivered after the runtime returns, including any scheduled dream;
streaming and separating that lifecycle remain subsequent roadmap work. Rendering
must avoid making a limited result look like an unconstrained successful answer
or claiming the reason for its limit state.
Tests cover lost activity, conflicting speaker activity and narrow terminals.

## Operational surface

This changes the local terminal of the existing `kuru` executable. It adds no
server, bind address, container, credential, provider call or target architecture.
The normal worktree preview remains on its native OpenAI provider; deterministic
fixture providers and isolated state are used for automated verification. Real
terminal checks cover practical wide and split-pane sizes. Native Windows checks
remain required in CI and are not claimed from this macOS host.
