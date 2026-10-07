# Tasks

## 1. Actual concurrent context

- [x] 1.1 Extend `apps/kuru-tui/tests/terminal.rs` so both concurrently admitted PTY sessions complete first turns with distinct per-part private deliberation outputs before publishing seeded prior-session typed summaries.
- [x] 1.2 Capture actual second-turn HTTP requests for every participating part in both still-live sessions; assert own live raw/private rows and same-actor approved summaries, rejecting foreign live raw/private, prior raw/reasoning, and unrelated-part private/summary markers.
- [x] 1.3 Retain actual stored-history, shared-owner, live-driver, EOF release and awaited memory cleanup assertions; run only the selected native terminal case and owning closing guard with required preparation.

## 2. Delivery evidence

- [x] 2.1 Run affected host/Windows lint, typecheck, format, documentation and managed checks; record observed outcomes and native platform limits.
- [x] 2.2 Prepare completed evidence for strict archive and Root-coordinated normal hooked commit and native CI delivery.

## Observed evidence

The owning app test task retained bundle preparation and the prepared memory
supervisor, then forwarded only the existing two-terminal case and closing guard.
Task 13604 exited 0: the terminal case passed in 13.09 seconds, the guard passed
in 0.05 seconds, and every other test was filtered. Two actual processes held
first provider requests concurrently on one managed Dolt service with distinct
live claims. Both first turns settled before synthetic continuity publication;
both second turns persisted while both live claims remained. Every part's captured
second deliberation HTTP request included its own live raw user row and private
deliberation output plus its typed same-actor prior-session summary. Assertions
checked the complete request for foreign live raw/private, synthetic prior raw
and private reasoning, and unrelated-part private and continuity markers. Existing
surviving-session, normal-close and abrupt-EOF claims and awaited cleanup passed.

Continuity summaries were seeded from a separate synthetic prior session using
public append, snapshot and checkpoint APIs under its own selected driver; that
driver and its attachments closed before the two actual live sessions continued.
This is real managed storage, session drivers, PTYs and HTTP serialization with a
fake provider, not paid inference or autonomous live summary generation.

Earlier construction failures are separate evidence: compiler 5290 rejected a
missing local parts binding and unsupported hex formatting; native task 65962
passed the guard but rejected unclaimed synthetic-session mutation. The corrected
fixture obtained its own claim; no production authority check changed. Initial
sandbox-only task attempts stopped before compilation on mbx permissions and a
taplo host SystemConfiguration panic; unchanged owning tasks succeeded through
normal escalation with process-local mise state/cache/trust paths.

Final scoped checks all exited 0: host lint 82013, Windows-target lint 68515,
all-target/all-feature typecheck 5647, docs build/content 30072, format 13977, and
managed drift 4655. No production source, dependencies, pins, tool versions or
configuration changed. Local behavior is macOS; this Unix PTY case's other native
platform execution and full PR/main CI remain delivery evidence to obtain. No
broad suite or coverage was repeated locally. Archive and physical verification
precede the final commit; commit waits for Root's shared Git window.
