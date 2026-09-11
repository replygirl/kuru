## 1. Stable native fixture identity

- [x] 1.1 Replace Unix hardlink aliases in `packages/kuru-connectors/src/test_support.rs` with symlinks to the immutable shared artifact; preserve independent plans, transcript paths, compiler ownership and cleanup.
- [x] 1.2 Extend the invocation regression to run through both the alias and the shared artifact and verify the actual peer reports one canonical executable while using its invocation-specific plan.
- [x] 1.3 Verify existing concurrent-plan isolation and sibling-retirement/final-owner cleanup regressions still exercise their real subprocess contracts without timing sleeps or retries.

## 2. Verification

- [x] 2.1 Run package-scoped connector tests, strict lint and formatting checks through mise; record observed results and retain any failure diagnostics.
- [x] 2.2 Revalidate the change strictly and report the scoped result for root integration and the full repository quality gate.

### Observed evidence

- The final invocation regression failed against the original hardlinks because
  the peer-reported image resolved to a disposable alias; it passed after the
  Unix symlink change. Both runs completed their exact wire plans, isolating the
  assertion from startup timing. On macOS, `current_exe` can report the invocation
  symlink, so the assertion resolves the reported path before comparing it.
- `mise run //packages/kuru-connectors:test`: all 33 tests passed, including
  concurrent-plan isolation, sibling retirement and final-owner cleanup.
- `mise run //packages/kuru-connectors:lint`: strict Clippy passed.
- `mise exec -- rustfmt --edition 2024 --check packages/kuru-connectors/src/test_support.rs`:
  passed. Strict cospec validation and the actual apply gate passed before edits.
- Local logs: `/tmp/kuru-stable-stdio-canonical-red.log`,
  `/tmp/kuru-stable-stdio-canonical-green.log`, `/tmp/kuru-stable-stdio-suite.log`
  and `/tmp/kuru-stable-stdio-lint.log`. The earlier bounded Gatekeeper diagnostic
  is recorded in `/tmp/kuru-connector-startup-diagnosis.md`.
- Repository-wide verification, archive and integration belong to the parent
  change owner; these package results do not claim that the full gate has run.
