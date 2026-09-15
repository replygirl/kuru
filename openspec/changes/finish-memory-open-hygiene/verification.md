## 1. Warm cache concurrency and integrity [critical]

- [x] 1.1 @regression (agent) hold the actual managed cache installation lock while opening its existing native runtime -> warm verification completes within a bounded deadline with full payload digests and the exact-version probe; this blocks before the fix
- [x] 1.2 @integration (agent) race actual concurrent warm opens against the same native runtime -> every open returns the same verified runtime without waiting for installation authority
- [x] 1.3 @regression (agent) corrupt the actual cached executable while retaining the expected size -> verification rejects it before the payload executes and preserves the cache
- [x] 1.4 @benchmark (agent) measure isolated actual cold and warm managed provisioning on the implementation host -> record honest wall times and retain full hashing without a threshold claim

## 2. Cold installation ownership

- [x] 2.1 @integration (agent) race missing-cache callers through the actual checked publication path -> exactly one verified identity is activated after the under-lock recheck
- [ ] 2.2 @unit (agent) cancel extraction/probe and exercise checked Windows activation uncertainty -> stage, payload handles and installation authority remain owned through real completion or bounded checked recovery

## 3. Unix privacy guidance

- [x] 3.1 @regression (agent) reject a current-user-owned 0755 data directory without legacy SQLite -> the exact path and mode-0700 remedy are reported before writes and no mode/content changes; this lacks the remedy before the fix
- [x] 3.2 @integration (agent) reject the same owner-owned directory with a legacy SQLite source -> guidance remains actionable and source bytes remain unchanged
- [x] 3.3 @unit (agent) reject linked and foreign-owned directory controls -> no chmod/mode-0700 guidance is emitted

## 4. Documentation and static checks

- [x] 4.1 @manual (agent) review memory documentation -> warm concurrency, full verification, cold serialization and narrow permission guidance match implementation
- [x] 4.2 @integration (agent) run package format, lint, typecheck and focused memory tests -> affected checks pass without weakening existing native controls

## Observed evidence

- macOS actual bundled-engine run: cold provisioning `3.049301s`; two concurrent warm opens under a held installation lock `136.433708ms`. Both warm callers retained full executable/license digests and exact-version probes; same-size executable corruption then failed with a checksum mismatch.
- Serial focused provision suite: 17 passed, covering cold contention, checked publication, corrupt cache/archive rejection, exact version, progress stages and cancellation ownership.
- Unix permission controls: ordinary owner-owned 0755, legacy owner-owned 0755, symlink and foreign-owner tests passed; the ordinary and legacy fixtures retained their modes and content.
- `mise run //packages/kuru-memory:typecheck` and `mise run //packages/kuru-memory:lint` passed. Owned Rust files were formatted with `rustfmt`; `git diff --check` passed.
- Windows-only activation uncertainty execution remains for native Windows CI; the existing checked recovery implementation and platform-specific tests were unchanged.
