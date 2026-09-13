## 1. Isolated published verifier

- [x] 1.1 Extract the existing native mise environment construction into one delivery-owned helper and verify the candidate-install fixture retains isolated roots, no ambient credentials or proxies, and its current behavior.
- [x] 1.2 Add the tooling-only `verify-published-windows` subcommand and bounded public GitHub release/tag/download validation and verify exact identity, inventory, checksum, archive, and selected executable mismatch cases.
- [x] 1.3 Implement ordinary isolated mise selection, activation, location and execution using the resolved absolute mise path and verify command plans contain `github:replygirl/kuru@VERSION` without endpoint replacements.
- [x] 1.4 Exercise the installed Kuru version, demo conversation/resume and memory status/history/sessions against cold offline data and engine caches, then verify extracted Dolt and licenses against the checked-out manifest.
- [x] 1.5 Await owned cleanup and write a bounded fixed-metadata JSON receipt only after isolated-root removal; verify informational stderr does not change stdout JSON parsing and raw child output is absent from evidence.

## 2. Package and release integration

- [x] 2.1 Add the delivery-owned `verify:published-windows` mise task and focused CLI/static tests, and verify it resolves native mise before environment clearing without adding memory or TUI crate dependencies.
- [x] 2.2 Add the post-publication `windows-2025` Release job at the exact bump SHA with setup-only GitHub token scope, artifact upload, and documentation dependency; verify workflow contract tests cover needs and credential boundaries.
- [x] 2.3 Update release operations and curated installation documentation and verify the automated gate, receipt, same-run rerun recovery, and compiler-free user path are accurate.

## 3. Acceptance and delivery

- [x] 3.1 Run focused delivery formatting, lint, typecheck, tests, workflow checks, and docs checks and record exact observed results in the verification ledger.
