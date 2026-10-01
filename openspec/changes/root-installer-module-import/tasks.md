# Tasks

## 1. Correct the source entrypoint

- [ ] 1.1 Generalize the portable PSHOME-import source contract and apply it to `scripts/install.ps1`, and verify it fails on the unfixed entrypoint and on an injected early `Split-Path`
- [ ] 1.2 Import the exact PSHOME Management and Utility manifests before the entrypoint's first non-Core command, and verify the portable contract passes
- [ ] 1.3 Add the native autoload-disabled source-entrypoint regression in `windows_cli.rs` without editing the existing probe, and verify it compiles under the Windows-target lint

## 2. Document and deliver

- [ ] 2.1 Update `docs/install.md` and `apps/kuru-docs/guide/installation.md` and verify docs checks pass
- [ ] 2.2 Run format, lint, Windows-target lint, typecheck, delivery tests and shell lint, strict Cospec validation/apply and normal hooks; record observed results and name unrun native Windows checks
