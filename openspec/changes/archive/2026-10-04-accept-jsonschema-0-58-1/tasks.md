# Tasks

## 1. Dependency acceptance

- [x] 1.1 Review the official Rust 0.58.0 to 0.58.1 source diff and the existing
  transitive 0.58.3 patches; verify compatibility with Kuru's selected features.
- [x] 1.2 Verify Dependabot's regenerated Cargo.lock with locked owning-package
  compilation and confirm that only the four intended package versions changed.
- [x] 1.3 Run the kuru-core owning test suite and verify published configuration
  schema acceptance, rejection, managed policy and native parser parity.
- [x] 1.4 Complete local acceptance with repository format, lint, typecheck,
  tooling, Cospec managed-drift and docs checks; record observed evidence.

Archive this completed record before the final branch commit. Merge requires
successful normal hooks and hosted checks for the final submitted head; local
acceptance does not claim a hosted or post-merge result.

## Observed evidence

- Official [Rust 0.58.0 to 0.58.1 comparison](https://github.com/Stranger6667/jsonschema/compare/rust-v0.58.0...rust-v0.58.1)
  shows a URI syntax optimization with parser-oracle tests. Existing
  `validator_for`, `Validator` and `is_valid` usage remains compatible.
- Official [Rust 0.58.0 to 0.58.3 comparison](https://github.com/Stranger6667/jsonschema/compare/rust-v0.58.0...rust-v0.58.3)
  shows unchanged regex implementation, Python-only value implementation changes
  behind features Kuru does not select, and referencing fixes for dynamic-anchor
  document bases and embedded schema vocabularies. The existing transitive patch
  versions satisfy jsonschema's upstream semver dependency requirements.
- On 2026-10-04, `MISE_LOCKED=1 mise run //packages/kuru-core:test` exited 0:
  90 passed, 0 failed, 0 ignored across eight targets, including all nine published
  configuration schema/native parser parity tests. Cargo compiled all four
  updated registry crates with `--locked`. `Cargo.lock` stayed byte-identical;
  its diff against the reviewed main base changes only those four versions and
  checksums. No lockfile regeneration beyond Dependabot's existing result was
  necessary.
- Separate `mise run format:code` and `mise run cospec:managed:check` checks
  exited 0. Strict validation and the apply gate both exited 0 before local
  acceptance began.
- Separate `mise run lint:rust`, `mise run lint:windows`, `mise run typecheck`,
  `mise run lint:tooling` and `mise run docs:check` checks each exited 0 on
  2026-10-04. Docs checks built the site and validated its public artifacts,
  local links and anchors. Native Windows behavioral acceptance and the combined
  coverage gate were not repeated locally for this scoped dependency acceptance;
  the final hosted workflow supplies those checks before merge.
