## 1. Private provenance

- [x] 1.1 Omit pre-turn rewrite provenance records from ordinary provider requests (current inputs and history, with the current-message count and source inventory taken after filtering) and from compaction rows, and verify with the platform regression test that fails when the filter is removed
- [x] 1.2 Document the durable private record and the rewritten-only provider view in `docs/configuration.md` and the docs site reference, and verify with `docs:check`

## 2. Hook cleanup and admission

- [x] 2.1 Share one post-exit cleanup deadline between reaping and the output drain, stop the drain on caller loss, and verify with the new connector test and the existing `hooks::` suite
- [x] 2.2 Route dream calls outside the offered dream tool through the shared pre-tool admission, and verify with the dream permission test
- [x] 2.3 Document that Windows stock PowerShell hooks should set `timeout_ms` for cold start (no `$PSHOME` bootstrap for hooks), and verify that the docs match AGENTS.md scope

## 3. Packaging and records

- [x] 3.1 Restore the macOS `strip -u -r` packaging-input flags in a separate commit, and verify that the TUI test target compiles
- [x] 3.2 Run focused checks (format, clippy, focused tests, docs), and record evidence and unrun checks in verification.md
