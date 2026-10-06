# Verification

## 1. Owned teardown and original error [critical]

- [x] 1.1 @regression (agent) compare macOS coverage job 112225031478's masked quiescence failure with the corrected real fixture -> the uploaded historical root panic remains red evidence; owning selection 12028 passed the unchanged successful fixture after owned cleanup. The missing historical error remains unknown.
- [x] 1.2 @integration (agent) deliberately return an error while the real migration opening/proxy is paused -> owning selection 12028 passed: the original deliberate error survives awaited task, proxy, store and lifecycle cleanup and root release; no unexplained engine/root guard error replaces it.

  Initial local owning selection 68293 returned 101: the original fixture
  passed, while this deliberate-error case preserved the original cause but
  failed the root guard because immediate cancellation did not await the
  creator's reap report. The corrected selection resumes and awaits the same
  opening within its existing budget before closing its returned store.

  Corrected owning selection 12028 returned 0: both cases passed in 4.72s,
  with 769 library cases and all other target cases filtered. Its normal
  package prerequisites prepared verified bundle fixtures and the supervisor
  snapshot. No exhaustive timeout/cancellation-path acceptance was run.

- [x] 1.3 @integration (agent) retain the original successful migration and reopen assertions -> owning selection 12028 passed unchanged assertions: actual durable reply is discarded, routed session ends, exactly one schema upgrade commit remains and reopen performs no migration replay.

## 2. Required scoped checks

- [x] 2.1 @integration (agent) run owning memory host/Windows lint, all-target typecheck, format, docs, managed and strict checks -> host lint 39874, Windows-target lint 62524, all-target/all-feature typecheck 59312, format 81179, docs 75919 and managed checks returned 0. Revised actual strict/apply 81438 returned 0 with all five contexts read. Remote PR and exact-main CI remain Product-owned and pending until observed.
