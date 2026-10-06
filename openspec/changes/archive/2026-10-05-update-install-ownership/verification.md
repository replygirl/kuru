# Verification

## 1. Manager-owned self-update refuses before effects [critical]

- [x] 1.1 @integration (agent) classify default/overridden mise and Homebrew roots, native aliases and near-miss path components -> exact manager/hint or ordinary installation, without string-prefix false positives
- [x] 1.2 @e2e (agent) run actual copied Kuru binaries from fake manager install paths in release and source modes with listener/build sentinels -> actionable refusal, zero outbound connections/build attempts, unchanged executable identity/bytes/inventory and no update-state directory
- [x] 1.3 @e2e (agent) invoke a symlink alias into a manager-owned executable -> resolved manager refusal before effects, original target untouched

## 2. Unmanaged checked installation remains safe [critical]

- [x] 2.1 @integration (agent) exercise real ordinary/read-only executable, hardlink/symlink/identity replacement and inaccessible-parent preflight -> valid 0555 file in writable parent accepted, unsafe destinations rejected without probe/staging files
- [x] 2.2 @regression (agent) run the meaningfully affected existing verified-install/source CLI cases -> current Unix success/output/input guarantees retained; Windows source/helper/receipt source untouched

## 3. Repository acceptance and ordered delivery

- [x] 3.1 @regression (agent) run owning host/Windows lint, typecheck, granular format/docs and structural fixture guards -> observed terminal outcomes on stable source, no pin or root-lockfile change
- [x] 3.2 @integration (agent) review final ownership call order/API and run strict/apply/context -> independent source/evidence clear and actual local tasks complete before archive
- [~] 3.3 @runtime (agent) native supported-platform previous-release/update acceptance and hosted90% coverage -> defer: run once at the appropriate integrated D2 delivery head; no cross-target or local-fixture native support claim
- [~] 3.4 @runtime (human) final release/publication -> defer: standing local implementation is authorized, remote PR workflow and final phase release remain separate; no release action in this local slice

## Observed outcomes

Actual initial strict validation and apply exited0 with a clear gate; all six returned contexts were read before implementation. Resolved-root classification, retained Unix UID/directory-access preflight and early CLI refusal are implemented. Refusal paths use escaped Path debug rendering. Filesystem preflight resolves the complete default/override root set, including default Cellar aliases; public classification stays pure. The held checked installation does not grant persistent mutation authority across network/build work. Original executable-parent spelling is forwarded to existing update modes. Existing Unix source shell delegation and Windows receipt/bootstrap code remain unchanged for this first slice.

Owning delivery selection33591 passed2/2: component-exact default/override/near-miss classification and actual held-file read-only/link/name/access checks. Non-root native execution also inspected, without executing or modifying, a foreign-owned stock image. Root jobs cannot establish the non-root refusal. The added real temporary aliased-root assertion initially used macOS's unresolved `/var` spelling (56999 failed1); correcting its resolved source coordinate gave59221 terminal0,1/1. No real manager directory was modified.

The first app invocation41989 accidentally combined Cargo `--all-targets` and `--test update`, which select a union. It completed171 library cases including the established closing guard, then was interrupted through its same owned task (terminal130) after unrelated CLI cases began; no update or whole-suite acceptance is attributed to that invocation. A subsequent read-only process-image/parent inspection found no Kuru/Dolt/test-CLI process; no numeric PID was signalled or held lock removed.

Corrected exact libtest-filter selection38902 ran only three update cases: the new copied-image case and existing verified-release case passed, while existing source forwarding exposed an unnecessary canonical destination spelling change. The new case performs six actual invocations (mise/Homebrew × release/source/alias), checks actionable fixed hints, empty stdout, unchanged inode/digest/inventory, absent build sentinel, empty HOME and zero connections to its listening release endpoint. All its new launches use platform-owned processes with bounded concurrently drained output through EOF, owned cleanup and reap. Restoring the former destination spelling retained the existing source assertion; only that failed case was rerun as38151, terminal0,1/1. No operational deadline or assertion was weakened. New-manager and verified-release passes were retained.

Owning host lints passed for platform25612, final delivery97283 and final app51757; Windows-target lints passed for platform18631, delivery87405 and final app15726. Platform56597/delivery94155 typechecks initially passed after correcting the mechanical rustix import to actual `accessat`; final app all-target/all-feature typecheck34834 passed. Final root format61370, full docs build/content/check51004 and managed drift16337 all exited0. The first format/docs checks identified only the new command-table format; owning formatting fixed it. Their simultaneous setup also emitted a transient hk config-lock warning, without any bypass; the sequential final docs setup/check succeeded. One package-owned pinned-tool installation refreshed only worktree mise metadata, restored byte-for-byte from its initially clean HEAD; root dirty lockfile and tool pins remain untouched.

Root and Architecture final independent source/evidence reviews are clear. Actual final strict/apply exited0 and all six returned context files were reread before archive; all six local tasks are complete. Archive, final commit/hooks, remote PR workflow, integrated native previous-release/update checks, hosted90% coverage, paid live smoke and Phase2 release are not preclaimed. Clean U5 and its actual archive remain intact at `ccad265c`.
