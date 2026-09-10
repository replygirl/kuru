## 1. Compiler-free installation [critical]

- [x] 1.1 @integration (agent) run package-owned bootstrap through real Bash with Cargo/Rust absent from PATH and a real packaged fixture -> 13 bootstrap tests passed in /tmp/kuru-bootstrap-tests-first.log; installed bytes/mode match the candidate, path spaces work and the candidate's execution marker remains absent
- [x] 1.2 @integration (agent) exercise explicit/latest resolution, all four host mappings and unsupported hosts using controlled transport/platform boundaries -> fixtures observed one latest manifest request followed by the selected immutable version URL, supported mappings passed, custom bases remained literal, CLI overrode environment defaults, and unsupported inputs failed before replacement
- [x] 1.3 @runtime (agent) install the actual published native archive outside the checkout and run version/demo/update with no compiler on PATH -> v0.1.0 reports its version, runs the seven-peer IFS demo and completes native self-update with unchanged expected bytes in /tmp/kuru-published-release.7P9FEJ. All four published archives also passed the trusted native validator and system Bash 3.2 bootstrap with matching executable bytes and a compiler-free PATH; the forwarding entrypoint and host demo passed, /tmp/kuru-published-archives.LUCjja/verification.log. Foreign binaries were validated and installed, not executed on the macOS arm64 host
- [x] 1.4 @runtime (agent) use isolated mise config/data/cache to install the actual published exact version -> mise 2026.9.3 installed github:replygirl/kuru@0.1.0 from the matching published macOS arm64 archive in 1.6 seconds with no compiler on PATH, and mise exec reported kuru 0.1.0. Evidence: /tmp/kuru-published-release.7P9FEJ/mise-install-auth-context.log and mise-kuru-version.txt. The credential helper retains gh's original HOME while all mise/Kuru configuration, data, cache and runtime paths remain isolated; the initial failed probe is preserved separately

## 2. Installation failure behavior [critical]

- [x] 2.1 @integration (agent) corrupt or omit assets, duplicate checksum entries, exceed manifest/download limits, fail transport and terminate the process during a blocked download -> passing real Bash fixtures preserve previous bytes and remove staging; SIGTERM exits 143 within the bounded wait and leaves neither producer nor other process-group children alive; valid bytes followed by transport failure still reject
- [x] 2.2 @integration (agent) supply duplicate, linked, traversal, missing entries, oversized expanded tar (including non-binary data), and symlink/directory destinations -> all reject with existing/outside bytes intact; a small valid GNU sparse member installs, while the same format above the logical extraction cap rejects even though its compressed archive is below 4 KiB

## 3. Documentation and repository checks

- [x] 3.1 @manual (agent) read README, both installation guides and built HTML structure -> direct mise/shell choices precede source; mise use/upgrade help confirms activation and exact-pin behavior; scoped wording scan found no visibility/readiness commentary. Docs build/link check passed after preserving the existing update-deliberately anchor. Independent review corrected a bare mise install reference by qualifying its maintainer-checkout context
- [x] 3.2 @integration (agent) run package tests, ShellCheck and full mise check -> final gate after source review passed in 46.68 seconds with 97.46% Rust line coverage (8993/9227), /tmp/kuru-direct-install-final-check.log. Earlier root test:install passed archive tests plus all 13 bootstrap tests, /tmp/kuru-direct-install-root-task.log. Docs build/anchors, formatting, Clippy, cospec and managed drift passed; repaired main aac594e integrated without conflict and the refreshed apply gate exited 0. Review clarified documentation about failures before atomic replacement; no further implementation change was needed
- [~] 3.3 @runtime (agent) observe Linux and macOS PR gates -> defer: hosted checks require the committed, archived branch; they will be observed and linked in the PR before its normal merge, without inferring success from local checks

## Prepublication artifact evidence

Release run 34424298823 produced all four native archives at prepared commit
7c38954f8e088333cef83bc6bfbb9dc60afb0ec1 for version 0.1.0. Each downloaded Actions
artifact passed its sidecar checksum, three-member inventory and trusted native
archive validation. The host arm64 macOS binary ran version, seven-peer IFS demo
and native self-update with an empty PATH; evidence is
/tmp/kuru-run34424298823-artifacts.AvX9dp/RESULTS.md.

The new Bash bootstrap then installed all four actual archives with system Bash
3.2, a compiler-free utility PATH and paths containing spaces; bytes matched the
native validator's output. The forwarding script and host demo passed too:
/tmp/kuru-bootstrap-artifact-smoke.vqTANL/verification.log. These are build-artifact
checks. They do not establish published-release, mise or Pages behavior.

## Published release and live site

Release 34432603265 passed all eleven jobs, published v0.1.0 at
aac594e8838921a2b34ba8adf762d73934a122b8, and completed its final inline Pages
deployment. The annotated tag peels to that exact commit. All four native archives
and SHA256SUMS were downloaded from the actual nondraft release and independently
validated; see the runtime evidence above and /tmp/kuru-release-sixth-completion.md.

The live homepage and all eighteen linked guides/assets returned successful HTTPS
responses and matched the deployed github-pages-1 artifact byte-for-byte:
/tmp/kuru-release-sixth-live-pages/verification.log. This is HTTP/content evidence,
not a fresh browser interaction audit. The site currently contains the initial
release's guide; this branch's revised guide deploys with a release containing it.

Actual asset acquisition uses the existing authenticated GitHub CLI. This does
not claim an anonymous shell download was exercised. Real Bash transport fixtures
cover latest and explicit HTTPS URL selection separately. The first isolated mise
probe reached a GitHub authentication error after all native runtime checks passed.
The same gh request succeeded with its original HOME and failed with the isolated
one. Retaining the original HOME only inside the credential helper corrected the
probe, and actual mise installation and version execution passed. No application
code, credential value, user configuration or verification setting was changed.
