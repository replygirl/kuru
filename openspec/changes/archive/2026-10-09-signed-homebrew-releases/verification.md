# Verification

## 1. Private signing boundary [critical]

- [x] 1.1 @integration (agent) Prepare from a hard-linked Cargo-style input, mutate only its signing copy and compare retained source/link bytes -> original bytes and identity remain unchanged.
- [x] 1.2 @integration (agent) Exercise existing, linked, unsafe, empty and oversized objects through checked preparation -> reject without replacing existing state.
- [x] 1.3 @runtime (agent) Sign a local macOS fixture ad hoc and run native verification -> structural signature exists but Developer ID production policy rejects it.

## 2. Signing release ordering and recovery [critical]

- [x] 2.1 @integration (agent) Exercise workflow contracts and exact retained artifact identity/digest fixtures -> final packages precede checksums/acceptance; retries reuse immutable bytes and conflicting state fails.
- [x] 2.2 @integration (agent) Exercise native verifier failure paths and missing configuration -> no unsigned packaging fallback; configured publisher and timestamp are required.
- [~] 2.3 @runtime (human) Observe a production Developer ID signature and Accepted Apple notarization result -> defer: maintainer explicitly plans account/credential setup after implementation; no production credentials available.
- [~] 2.4 @runtime (human) Observe Azure Public Trust signatures on x64/ARM64 and native exact-byte acceptance -> defer: maintainer explicitly plans Azure identity/profile/OIDC setup after implementation; no production credentials available.

## 3. Homebrew inventory and publication [critical]

- [x] 3.1 @integration (agent) Generate formulas from complete and malformed candidate manifests -> exact paired target assets only; missing/invalid digests fail without rewriting source manifests.
- [x] 3.2 @integration (agent) Use a real local HTTP GitHub fixture for public release identity, tap Contents API and compare-and-swap -> identical retry performs no write; rollback, same-version conflict and concurrent change fail.
- [~] 3.3 @e2e (agent) Run native Homebrew install/test/upgrade acceptance with generated formula -> defer: final whole-fixture result awaits required final-head PR CI before merge; native product assertions pass on all three supported hosts, but the correction that removes retained predecessor kegs and proves an empty fixture Cellar still needs native CI. Local Homebrew is blocked by outdated Command Line Tools. Final results are maintained in the private Phase 2 working document.

## 4. Repository integration [critical]

- [x] 4.1 @integration (agent) Run delivery tests, relevant lint/format/type checks, tooling, docs and strict cospec validation -> required local checks pass with no dependency/tool pin drift.
- [~] 4.2 @integration (agent) Run all PR CI checks and exact merged-main checks -> defer: the archived change must first be committed and pushed; every exact-final-head PR check must pass before merge, followed by every exact merged-main check. This remains required work in the authorized workflow; results and any follow-up fixes are maintained in the private Phase 2 working document.

## Observed local evidence

The full delivery suite passes, including 292 unit tests and every applicable
integration target. The final focused release contracts pass 66 tests after the
fixture cleanup correction: real CLI input preservation and closed inventories,
HTTP retry/compare-and-swap, signing/notarization failure cleanup and native macOS
ad hoc production-policy rejection. Host/Windows-target lint, typecheck, format,
shell/tooling and documentation build/content/link checks pass. Both implementation
commits and pushes passed normal hk hooks without dependency/tool pin drift.

Source CI 38005059545 on `e33c829` observes native Homebrew v0.10.0-to-v0.11.0
upgrade on macOS, Linux x64 and Linux ARM64. All three verify exact executable,
documents, four completions and man; genuine previous-release durable resume;
formula test; cold offline conversation, reopen and process cleanup; and native
self-update refusal before effects. Each then fails final untap because ordinary
uninstall removes only the latest keg while the fixture retains its predecessor.
The correction uses documented `uninstall --force` for this exclusively owned
formula and asserts its entire Cellar directory is absent before untap. The
pre-existing-install refusal and process-quiescence gates remain intact. Complete
fixture success after this correction is pending mandatory final-head native CI,
not claimed here. Ubuntu workspace coverage already passes on that prior head.

The dedicated tap PR #1 and exact tap main `8ad8f707` both pass their three native
formula checks; bootstrap uses verified immutable public v0.11.0 assets. Local
Homebrew installation was refused before installation by outdated Command Line
Tools; its owned fixture tap was removed and system tools were left alone.
Production Developer ID/notarization and Azure Public Trust acceptance remain
explicitly deferred to the maintainer's later credential setup. No release was
dispatched. Final source PR/main results are tracked in the private working doc.
