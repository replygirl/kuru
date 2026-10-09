## 1. Checked signing helpers

- [x] 1.1 Implement private bounded signing-copy preparation and native publisher verification in delivery tooling.
- [x] 1.2 Verify hard-linked build-input preservation, unsafe object rejection, native verifier failures and macOS ad hoc rejection.

## 2. Signed release automation

- [x] 2.1 Add preflight, ephemeral Apple identity/notarization, Azure OIDC signing on Windows x64 for both architectures, and verification before final packaging.
- [x] 2.2 Retain and validate exact source/version/target packages across retries without overwriting signed or public artifacts.
- [x] 2.3 Verify workflow ordering, retained artifact failure/reuse and native staged acceptance contracts with focused tests.

## 3. Homebrew delivery

- [x] 3.1 Generate a complete binary formula with paired support payloads and implement idempotent public-release-bound tap updates through a separate scoped app.
- [x] 3.2 Add native install/test/upgrade, exact byte, offline runtime/cleanup and self-update refusal acceptance to supported release legs.
- [x] 3.3 Verify formula inventory, read-only manifest generation and GitHub HTTP publication failure/retry contracts.

## 4. Documentation and integration

- [x] 4.1 Update owning installation/release/development documentation and the private Phase 2 working document; identify deferred live account acceptance accurately.
- [x] 4.2 Run relevant local checks and strict cospec validation; archive through the CLI before the final branch commit, with final-head CI and merged-main acceptance explicitly retained as subsequent workflow gates.
- [x] 4.3 Prepare the dedicated tap bootstrap and normal PR workflow with production account prerequisites explicitly identified.
