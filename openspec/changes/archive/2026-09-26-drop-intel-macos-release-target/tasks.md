## 1. Canonical specs

- [x] 1.1 Replace the fixed "five" archive, inventory and target counts in `release-automation` and `repository-delivery` with release-catalog wording through MODIFIED deltas, and verify with `mise run cospec -- validate drop-intel-macos-release-target --strict` and, after archive, `grep -n five openspec/specs/release-automation/spec.md openspec/specs/repository-delivery/spec.md` returning no archive-count claims
- [x] 1.2 Record the Intel installer refusal and catalog membership as an ADDED `repository-delivery` requirement, and verify the merged requirement exists in `openspec/specs/repository-delivery/spec.md` after archive

## 2. Documentation

- [x] 2.1 Add the Intel-after-v0.9.0 paragraph with the v0.9.0 tag bootstrap command to `docs/install.md` and `apps/kuru-docs/guide/installation.md`, and verify with `mise run docs:check` exiting 0

## 3. Test fixture label

- [x] 3.1 Drive the release-workflow native-gate test with `ubuntu-latest` instead of the retired `ubuntu-24.04` label, and verify with `mise run //packages/kuru-delivery:test` exiting 0
