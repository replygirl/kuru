## 1. Exact published acceptance

- [x] 1.1 @regression (agent) actionlint and workflow review -> passed `mise run lint:workflows`; the job depends on publish, checks out `needs.bump.outputs.sha`, has `contents: read`, passes exact version/SHA inputs, and uploads the runner-temp receipt.
- [~] 1.2 @runtime (agent) GitHub Actions windows-2025 on the first authorized main Release run containing this change -> defer: the manual main-only Release workflow has not yet run; its public download, cold offline reopen, and receipt are required live evidence.

## 2. Documentation and discipline

- [x] 2.1 @regression (agent) docs formatting/build/link checks and strict Cospec validation -> passed `mise run docs:check` and `mise run cospec -- validate verify-published-release --strict`; documentation separates publication from public verification and directs same-run retry.
