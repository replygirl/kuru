## Context

`View::new(&Harness, models).await` currently reads public transcript history,
session identity and turn count, selected mode/model/effort, active topology,
live relationships, focus, the project label from `Harness::cwd`, and
`KURU_REDUCED_MOTION`. `View::refresh(&Harness)` rereads only mutable runtime
facts and then removes activity and routes for parts that are no longer active.
The renderer and scene already consume only `View`, but their fixtures must
construct a real `Harness` and provision Dolt because construction and refresh
cross that boundary.

The run loop also carries behavior that this extraction must preserve. A
completion is generation-checked, activity is drained, and the typed
`DispatchOutcome` updates the transcript or command feedback before mutable
runtime state is refreshed and settled. Failed dispatches still refresh.
Cancellation aborts and awaits the job, drains activity, advances the
generation, clears busy state, settles, refreshes, clears its prior notice, and
only then publishes the cancelled status and completion lock.

The archived `authoritative-turn-display` change made `TurnOutput` the source
of completed answer text, speaker, relationship, usage, and limited-result
metadata. This refactor consumes that established seam without changing it.

## Goals / Non-Goals

**Goals:**

- Make `View` construction and mutable refresh synchronous operations over
  owned presentation data.
- Keep all Harness, history, environment, path, and other I/O reads in a TUI
  adapter owned by the run loop.
- Run render, editor, scene, and view-state tests from plain fixtures without
  provisioning or starting Dolt.
- Preserve the exact completion, failure, generation, refresh, settlement, and
  cancellation ordering already exercised by the TUI.
- Retain a real-Harness integration target proving that the production adapter,
  relationship route, and typed completed result reach the view correctly.

**Non-Goals:**

- No streaming, `EventStream`, new event model, new cancellation policy, or
  runtime ownership change.
- No public `TurnOutput`, `run --json`, CLI, provider, memory, session,
  persistence, configuration, or schema change.
- No redesign of part labels, `Relationship`, transcript tuples, completion
  metadata, activity decoration, renderer caches, or status wording.
- No removal or substitution of real PTY, preference, trust, persistence,
  provider, or native terminal coverage.

## Decisions

### Use two owned inputs with different lifetimes

Add application-local inputs with the existing field representations:

```rust
InitialViewData {
    transcript: Vec<(String, String)>,
    session: String,
    project: String,
    motion: bool,
    runtime: RuntimeSnapshot,
}

RuntimeSnapshot {
    turns: usize,
    mode: String,
    model: String,
    effort: String,
    parts: Vec<(String, String)>,
    relationships: Vec<Relationship>,
    focus: Option<String>,
}
```

`View::from_initial(initial, models)` initializes presentation state, and
`View::apply_runtime(snapshot)` updates mutable runtime facts and filters stale
part activity and routes. Both are synchronous and perform no Harness, memory,
history, path, clock, or environment reads. The mutable snapshot cannot assign
transcript, session, project, or motion, so an ordinary refresh cannot replace
completed text or initial identity.

The alternative is an async constructor backed by `Harness`, a trait facade, or
a closure that performs reads. That preserves hidden I/O in the presentation
layer and keeps plain tests coupled to runtime setup, so it is rejected.

Keep `parts` as the current `(id, "name · role")` tuples, relationships as the
current core values, and completion metadata as the private
`BTreeMap<usize, String>`. Introducing typed labels or a new completion record
would combine a second presentation redesign with this extraction.

### Project runtime truth once in the TUI adapter

The TUI adapter provides one async initial projection and one synchronous
runtime projection. Initial projection reads `Harness::history().await`, the
session and project identity, the reduced-motion environment setting, and the
same mutable facts used by `RuntimeSnapshot`. Runtime projection reads only the
current Harness configuration, turn count, active parts, live relationships,
and focus. The run loop calls the adapter while it owns the appropriate Harness
reference or lock, then releases that runtime access before applying the owned
value to `View`.

These projection functions remain in the TUI crate and are exposed only as
needed by its explicit integration target. They do not become core/runtime
contracts. Model catalog data remains the existing owned `Vec<ModelInfo>`
supplied by the TUI caller.

The alternative is to hand-build snapshots at every call site or in tests.
That can let synthetic fixtures pass while production forgets a field, so one
canonical adapter is required and directly exercised against a real Harness.

### Preserve the run-loop order literally

For a completed dispatch, reject a mismatched generation before draining or
applying anything. For the accepted generation, drain activity, clear busy
state and the job, then handle the outcome:

- A turn calls `complete_turn(TurnOutput)` before projecting or applying the
  mutable runtime snapshot.
- A successful command appends feedback or the saved-preference notice and
  locks completion before projecting persisted mode/model/effort and topology.
- A failed dispatch appends the error row, marks failed and completion-locked,
  then still projects and applies mutable runtime state before settling.

After any accepted outcome, lock the Harness, create `RuntimeSnapshot`, apply
it, and call `settle`. The snapshot's field limits reinforce the ordering by
making transcript replacement impossible.

Cancellation retains its distinct current sequence: abort and await the job;
drain activity; increment the generation; clear quit-pending and busy; settle;
project and apply the mutable snapshot; clear the notice; then set
`Cancelled · turn interrupted` and the completion lock. A completion tagged
with the prior generation is discarded before `complete_turn` or snapshot
projection.

The alternative is a common refresh-before-outcome helper or reordered cancel
cleanup. Either can expose stale persisted selections, freeze topology after a
failure, admit a cancelled completion, or let refresh overwrite settled status,
so this refactor keeps the branches explicit.

### Split plain presentation coverage from runtime projection coverage

Replace the Harness-backed fixtures in `src/ui.rs`, `src/ui/scene.rs`, and
`tests/visual.rs` with `InitialViewData` and `RuntimeSnapshot` fixtures. Their
rendered Ratatui-cell, editor, picker, activity, metadata, animation, wrapping,
narrow/wide, and scene assertions remain the same. These tests must not call
`Harness::new`, `MemoryStore::temporary`, provisioning, or server startup.

Move the real relationship and peer-route scenario into a distinct integration
target, expected as `apps/kuru-tui/tests/ui_runtime.rs`. That target constructs
the real deterministic Harness and memory fixture, calls the production initial
and mutable adapter functions, and compares their transcript, session, turn,
selection, active-part labels, relationships, focus, and project facts with the
Harness that produced them. It then creates a real relationship, runs the
deterministic turn, applies returned completion and sanitized route activity,
and verifies the projected relationship members, route, returned text, speaker,
and completion metadata reach the rendered view. The real slash-command state
scenario lives in the separate `src/ui/runtime_tests.rs` test-only module,
which can exercise the existing private dispatch function. Plain `ui::tests`,
scene and visual fixtures remain independent of Dolt; dispatch does not gain a
public API merely for tests.

This adapter regression is required even when every synthetic view fixture
passes. The alternative of retaining only synthetic relationship/events tests
does not prove that runtime truth crosses the production adapter and is
rejected.

Existing Unix and Windows PTY, preferences, trust, persistence, packaged
runtime, and provider tests remain in place. Building the TUI still follows the
same compile-time verified bundled-engine preparation; removing a runtime Dolt
process from plain tests does not relax that build requirement.

## Risks / Trade-offs

- **[Risk] Snapshot application drifts from current dispatch ordering.** → Add
  focused run-loop characterization for successful turn, preference command,
  failed dispatch, cancellation, and stale generation, asserting the relative
  completion/apply/settle outcomes.
- **[Risk] Plain fixtures hide a production projection omission.** → Keep the
  named real-Harness adapter integration target and compare all projected facts
  with their runtime source before rendering the relationship/route scenario.
- **[Risk] Inactive topology leaves stale visual activity or routes.** → Keep
  filtering inside `apply_runtime` and preserve its existing removal tests with
  explicit snapshots.
- **[Risk] Faster view tests are mistaken for a packaging change.** → Retain
  package-owned bundle preparation, real-memory integration tests, and the
  existing compile-time engine requirements unchanged.
- **[Trade-off] Application-local projection types add a small public test
  seam.** → Keep their fields owned and presentation-specific, and do not move
  them into `kuru-core` or `kuru-runtime`.

## Seam ownership

- `Harness` continues to own session, configuration, topology, relationships,
  history, memory, provider work, and reconciliation.
- The TUI adapter owns all reads from Harness, history, environment, and project
  paths and converts them into owned initial or mutable presentation data.
- `run_loop` continues to own event subscription, Harness locking, dispatch
  jobs, generation checks, cancellation, ordering, and application of adapter
  output.
- `View` owns editor, picker, transcript presentation, completion metadata,
  status, notices, animation, activity decoration, routes, and scene state. It
  receives runtime facts but performs no runtime or I/O reads.
- `TurnOutput` remains the runtime-owned authoritative completion returned
  through the existing TUI-local `DispatchOutcome`; broadcast events remain
  view-local sanitized decoration.

## Operational surface

The surface remains the local terminal of the existing `kuru` process. This
refactor adds no bind address, listener, container, runner service, secret,
credential read, connection limit, executable, target architecture, or runtime
dependency. Provider and Dolt processes remain confined to the existing real
integration and PTY paths. The ordinary binary still embeds and verifies the
same pinned Dolt asset for its target; plain view tests merely stop launching
that engine at runtime.
