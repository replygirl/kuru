## 1. Owned startup recovery

- [x] 1.1 Retry only an exact fresh selected-port collision after typed premature exit, complete owned reap and drain, with at most three attempts under the original startup deadline and lifecycle lease.
- [x] 1.2 Add a deterministic real-Dolt selected-port takeover regression that fails before this fix and succeeds after while the unrelated holder remains live.
- [x] 1.3 Add persistent collision, generic premature exit, and cancellation/parent-close controls that prove no extra dispatch, endpoint publication, or lifetime leak.
- [x] 1.4 Run focused package checks, record actual local evidence and native CI pending status, validate and archive this change.
