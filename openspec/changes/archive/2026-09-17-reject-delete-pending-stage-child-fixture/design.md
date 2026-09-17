## Context

`reconcile-denied-private-stage-delete` widened `recoverable_child_removal` so a rejected private-child removal reporting native error 5 enters the existing two-second cleanup window, and added `files::tests::private_temp_retries_a_denied_child_delete_after_the_holder_releases` to hold that behavior. That fixture had never run anywhere before it reached native CI, and it is wrong.

`delete_pending_private_file` opened the blocking child once with `FILE_FLAG_DELETE_ON_CLOSE` and `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE`. A delete-on-close handle does not put the name into the delete-pending state while it is open; it applies the disposition at close, and its shared delete access means every later open still succeeds. So `remove_children` opened, pinned and deleted the child normally, set `removed = true`, and the child name survived only because the fixture handle still referenced it. `remove_empty_directory` then failed on the still-occupied stage, and the `removed` flag mapped that failure to `PublicationPhase::Uncertain`. The test asserted `Rejected`.

That path is deterministic, and the native evidence shows it: runs 35235509039, 35235507600, 35235510132, 35235507684 and 35235507906 each report exactly one failure, this fixture, with the identical `left: Some(Uncertain)`, `right: Some(Rejected)`. It is also already covered — `private_temp_reconciles_pending_nested_delete_after_the_holder_releases` asserts `Uncertain` for the same shape with a held descendant directory — so the new fixture was silently duplicating an existing case rather than reaching the rejected one.

The same runs show the three originally failing Windows tests now passing, so the widened predicate is doing its job in production and must stay.

## Goals / Non-Goals

**Goals:** Make the regression fixture actually produce a rejected native error 5 before anything is removed; make its premise self-checking; leave a permanent diagnostic that distinguishes a blocked native call from a fast spin when the bounded window exhausts.

**Non-Goals:** No change to the recovery predicate, the two-second bound, the retry spacing, the identity checks, `PublicationPhase` semantics, any platform API, or any deletion, publication or process policy. No weakened assertion and no deleted coverage.

## Decisions

The fixture now creates the delete-pending state the way Windows actually reaches it: open a retained `GENERIC_READ` handle sharing read, write and delete, then open and immediately close a second handle carrying `DELETE` and `FILE_FLAG_DELETE_ON_CLOSE`. Closing the second handle applies the disposition, and because the retained handle still references the object the entry stays in the namespace as delete-pending. Every later open, including the checked pin in `remove_children`, is then refused with `ERROR_ACCESS_DENIED`, nothing is removed, and `remove_tree` reports `Rejected` — the exact case the predicate widening exists for. Dropping the retained handle in the observer releases the name inside the same bounded window.

The fixture asserts its own premise with an ordinary `File::open` that must fail with raw error 5. Rust's default share mode includes delete, so this open would succeed if the name were not truly delete-pending; a future change in native semantics therefore fails at the fixture, naming the premise, rather than at the outcome assertion. The outcome assertion gains a message saying what an uncertain result would have meant, which is the sentence that would have shortened this diagnosis.

`wait_for_cleanup_retry` now reports the reconcile attempt count and the real elapsed window alongside the retained first cause. Both a single native call that blocked past the deadline and a tight spin exhaust the same two seconds, and nothing in the current message separates them. Each native iteration costs a full CI cycle, so this is recorded permanently and only on the terminal error path.

## Risks / Trade-offs

The fixture now depends on delete-pending semantics rather than on sharing semantics. That dependency is explicit and asserted, so a divergence is reported at the fixture instead of as a confusing phase mismatch. Threading an attempt counter touches every retry call site in `close_windows_private_stage_with`, which is mechanical and changes no control flow. The exhaustion message grows by two values; it is emitted once, on failure.

## Operational surface

Windows-only private-stage cleanup fixtures inside the managed engine cache, and the text of one post-publication cleanup error. No bind address, secret, bundled asset, binary version, arch or CI topology changes.
