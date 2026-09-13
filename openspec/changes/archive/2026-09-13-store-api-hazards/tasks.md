## 1. Explicit shared lifecycle

- [x] 1.1 Add the real pre-fix close/clone regression and record its observed
  failure before changing the public API.
- [x] 1.2 Make `MemoryStore::close` consuming, update only necessary explicit
  cleanup call sites, and document shared-server shutdown versus ordinary drop.
- [x] 1.3 Route public reads through the existing closed-pool check and prove
  retained clones get the same closed-store error for reads and writes.

## 2. Durable results and graph inspection

- [x] 2.1 Expose `MemoryStore::reconcile` as `Result<Option<bool>>`, preserve
  pending-publication checks, and prove lost-reply committed/uncommitted states.
- [x] 2.2 Use pinned-Dolt graph order plus a stable hash tie-break for revisions
  and prove tied-date ancestry ordering with a bounded real-Dolt fixture.

## 3. Evidence and documentation

- [x] 3.1 Update owning memory documentation with lifecycle, reconciliation,
  and deterministic revision semantics.
- [x] 3.2 Run and record focused memory tests, static/docs checks, retaining
  native Windows and integrated coverage as pending gates.
