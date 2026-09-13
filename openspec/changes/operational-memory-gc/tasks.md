## 1. Bounded operation receipts

- [x] 1.1 Replace prior active receipts inside the existing mutation transaction after uncertainty reconciliation and verify repeated writes retain one current receipt and all revisions
- [x] 1.2 Exercise lost acknowledgement followed by another mutation against real Dolt and verify reconciliation precedes receipt replacement without replay

## 2. Explicit candidate lifecycle

- [x] 2.1 Add strict ordinary/promoting/abandoned branch identities, owned candidate-pool retirement and exact two-ref observation without adding a schema table; verify dirty, mismatched and uncertain states preserve every ref
- [x] 2.2 Move promotion through the non-force status transition and keep confirmed main authority independent of cleanup; verify copy/delete reply loss, repeated promotion after view closure and exact target publication
- [x] 2.3 Add explicit accepted-worker abandonment plus fixed-batch conservative startup cleanup; verify only eligible explicit abandoned or already-promoted refs are reclaimed, ineligible refs do not block main, and startup never performs a pending merge

## 3. Runtime resolution

- [x] 3.1 Settle accepted dream writes before explicit abandonment on cancellation or failure and verify live/candidate isolation plus later-turn reuse
- [x] 3.2 Preserve promotion-winning runtime publication when candidate cleanup is incomplete and verify exact topology, revision and repeated result

## 4. Engine GC and documentation

- [x] 4.1 Enable pinned Dolt automatic GC in generated server configuration, retain bounded private diagnostics and verify the effective SQL setting plus actual session-aware GC behavior
- [x] 4.2 Document receipt, candidate and GC behavior in both owning memory guides, including no automatic expiry, reachable-history growth and no secure-erasure claim; verify curated content checks

## 5. Coordinated acceptance

- [x] 5.1 Run focused real-Dolt receipt, candidate, runtime cancellation, GC, Rust format, lint and typecheck checks; record exact observed outcomes in the verification ledger
- [ ] 5.2 Run the single coordinated workspace coverage task and native macOS/Linux/Windows lifecycle checks; retain the 90 percent floor and record unavailable native evidence honestly
