## 1. Preserve repository conventions

- [x] 1.1 Audit and correct AGENTS.md against package task ownership, Cargo boundaries, hooks and cospec commands; verify examples against checked-in configuration and the independent review.
- [x] 1.2 Reinforce release-only Pages, archival-before-merge, canonical instructions and scoped delegation; verify alignment with the completed release workflow and existing user priorities.
- [x] 1.3 Run the full repository and managed-drift gates, record their actual results, and archive through cospec before the final branch commit.

Observed evidence: root and independent read-only review checked mise.toml,
packages/kuru-delivery/mise.toml, hk.pkl, generated harness configuration and the
inline release jobs. The guide now uses task-scoped cospec commands, states gate
exit meanings and typed artifact sizing, preserves apps/packages > mise > Rust >
other tooling, and keeps release/notes tools scoped to their owning tasks.
CLAUDE.md remains exactly @AGENTS.md. No version or test-count snapshots were
added to the guide. mise run check exited 0 on 2026-09-09: 209 tests, 97.51%
coverage, all lint/format/docs checks and cospec managed-file drift checks passed.
