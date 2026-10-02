# Tasks

## 1. Expose the product's own cleanup bound for test use

- [ ] 1.1 Add a `#[cfg(any(test, feature = "test-support"))]` accessor on
  `HookHost` in `packages/kuru-connectors/src/hooks.rs` that returns the
  `quiesce()` cleanup bound (today the private `QUIESCE` constant), mirroring
  the existing `in_flight_hooks()` visibility pattern at `hooks.rs:447`, and
  re-export it from the crate the same way `in_flight_hooks` is consumed by
  `kuru-runtime`'s tests. Verify by compiling `kuru-runtime` tests against
  the new accessor with no change to `quiesce()`'s behavior or value
  (`cargo doc`/`cargo check -p kuru-connectors` shows the symbol; `cargo
  check -p kuru-runtime --tests` resolves the import).

## 2. Re-derive the marker wait from the pipeline's actual stated budgets

- [ ] 2.1 In
  `cancelled_dream_abandons_candidate_hook_annotations_and_reaps_hook_descendants`,
  subscribe to the harness's event stream before spawning the dream task,
  and replace the first half of the flat 10-second marker wait with an
  event-driven wait (bounded only by a generous deadlock backstop, not a
  number raced against the pipeline) for the first post-tool hook's own
  `"annotated"` observation — the one externally visible signal that
  `reconcile`, `acquire_dream_lease`, `begin_candidate`, the fake provider's
  completion, and the first hook's dispatch/settle/`finish_post_tool_hooks`
  have all cleared. Verify by running the test under `-- --nocapture` and
  confirming the intermediate wait resolves on the hook's "annotated" event,
  not on a fixed sleep.
- [ ] 2.2 Bound the remaining, genuinely budgeted step — dispatch of the
  second tool call and the launch of its shell hook up to the marker write —
  by that hook's own configured `timeout_ms` (read from the `HookCommand`
  actually constructed for the test, not a duplicated literal) plus one
  named, documented scheduling margin, replacing the bare `Duration::from_secs(10)`.
  Verify by reading the derivation comment states the two terms and their
  source, and that the bound is computed from the `HookCommand` value rather
  than restating `5_000`.
- [ ] 2.3 On expiry of either wait in 2.1/2.2, report diagnostics captured
  before the dream task was spawned: the harness's step-timing marks (`"dream
  started"`, `"reconcile finished"`, `"dream lease acquired"`, `"dream
  candidate begun"`), the hook host's `in_flight_hooks()` count, and
  `marker`/`survived` file existence. Verify by reading the panic message
  includes all four pieces of state.

## 3. Re-derive the dream-join wait from the product's own quiesce bound

- [ ] 3.1 Replace the dream-join wait's flat `Duration::from_secs(10)` with a
  bound computed from the new accessor added in 1.1 (the product's `quiesce`
  cleanup bound) plus a stated margin, so the test's own wait can no longer
  tie the product's documented worst case. Verify by reading the bound is
  expressed as `<accessor>() + <named margin>`, not a bare literal, and that
  the margin's derivation is stated in a comment.

## 4. Confirm the fix without manufacturing a false pass

- [ ] 4.1 Run `cancelled_dream_abandons_candidate_hook_annotations_and_reaps_hook_descendants`
  locally and record the observed wall-clock split across the two re-derived
  marker-wait phases and the join wait, to confirm the new bounds have real
  headroom over what was actually observed, not just over the old flat `10`.
  Record the observed numbers as evidence; do not claim this reproduces the
  original CI race (a loaded, instrumented, multi-partition macOS coverage
  runner is not reproducible locally) — record that limitation explicitly
  rather than asserting a deterministic repro.
- [ ] 4.2 Run the full `kuru-runtime` test suite (`mise run
  //packages/kuru-runtime:test`) to confirm no other test depends on the
  changed waits' old timing, and record the pass/fail evidence.
