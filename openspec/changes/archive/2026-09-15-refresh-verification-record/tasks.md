## 1. Current verification record

- [x] 1.1 Document the current quality/native/Windows shard graph and final
  pre-publication release gates in `docs/verification.md`; verify each described
  job and dependency against the committed workflow files and scope the cited
  successful runs to v0.3.2.
- [x] 1.2 Run `mise run docs:check` and strict Cospec validation; verify the
  documentation build, content/link checks, and change validation pass.

Observed 2026-09-15: `mise run docs:check` exited 0, including VitePress build,
documentation formatting/lint, and generated-site content/link checks. Strict
validation exited 0 with no errors or warnings. Hosted links describe v0.3.2,
not unrun acceptance of the concurrent Phase 0 implementation changes.
