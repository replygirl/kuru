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
- [ ] 3.3 @e2e (agent) Run native Homebrew install/test/upgrade acceptance with generated formula -> installed executable/support bytes match, offline conversation/reopen and cleanup succeed, self-update refuses before effects.

## 4. Repository integration [critical]

- [ ] 4.1 @integration (agent) Run delivery tests, relevant lint/format/type checks, tooling, docs and strict cospec validation -> required local checks pass with no dependency/tool pin drift.
- [ ] 4.2 @integration (agent) Run all PR CI checks and exact merged-main checks -> all required checks pass; follow up on legitimate failures before reporting complete.

## Observed local evidence

The full delivery suite passes, including 292 unit tests and every applicable
integration target. The final focused release contracts pass 66 tests, covering
real release CLI children, exact package inventories, read-only formula generation,
HTTP retry/compare-and-swap and signing/notarization failure cleanup. Native macOS
ad hoc production-policy rejection passes. Delivery host/Windows-target lint,
typecheck, shell/tooling checks and the documentation build/content/link checks
pass; final commit/push hooks will validate the complete branch again.

Local native Homebrew installation fetched verified v0.10.0 payloads but stopped
before installation because Homebrew rejected this host's outdated Command Line
Tools. No Kuru was installed; the owned fixture tap was removed. The fixture now
runs in ordinary native Linux x64/macOS installation CI and Linux ARM build CI,
with the actual native executable and explicit foreign archive-shape fixtures.
The Release workflow retains the complete real candidate route. Native Homebrew
and exact branch CI results remain pending; they are not claimed as passed.

The initial source PR run 38003428408 confirms Linux x64/ARM64 previous-release
installation, exact installed bytes and offline durable reopen, then fails in the
fixture at an unsupported `git pull`: its deliberately childless Git audit rejects
Git's transport children. The fixture now commits the candidate formula directly
into its owned tap before the real Homebrew upgrade; the shared audit remains
unchanged. Complete install/upgrade acceptance is still pending the corrected CI.
Formula CLI regressions also prove create-only output cannot truncate a verified
manifest/archive through a direct path, hardlink or symlink; 66 focused tests pass.
