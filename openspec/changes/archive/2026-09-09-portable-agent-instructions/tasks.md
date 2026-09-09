## 1. Shared instructions and generated workflows

- [x] 1.1 Move repository instructions into AGENTS.md, import them from CLAUDE.md, and update the tooling invariant; verify the canonical body is preserved and `mise run lint:tooling` passes.
- [x] 1.2 Generate Claude Code, Codex and OpenCode harness files using cospec init with the existing commit gate preserved; verify `mise run cospec:managed:check` detects all three harnesses without drift.
- [x] 1.3 Document the canonical instructions and regeneration workflow in contributor/development docs; verify the documented generated paths and command syntax match actual output.

## 2. Integrated validation

- [x] 2.1 Run the full repository gate after the concurrent CI repair lands, record observed evidence, and validate this chore strictly before archive.

## Acceptance ledger

| Acceptance | Verification | Evidence |
| --- | --- | --- |
| All assistants read the same maintained instructions | Compare canonical AGENTS body with the previous CLAUDE body, allowing the updated instruction-source explanation; validate the import invariant | Verified body preservation with only source/generation instructions updated; CLAUDE contains exactly `@AGENTS.md`; metadata invariant passes |
| Claude Code, Codex and OpenCode receive cospec workflows | Tool-generated file tree and `mise run cospec:managed:check` | Generated 37 files: 12 shared Codex skills, 12 OpenCode commands, 12 OpenCode skills and one Codex rule file; existing Claude files retained; update JSON detects all three harnesses with every file unchanged |
| Existing repository gate remains enforced | `mise run lint:tooling` and full `mise run check` | Targeted tooling gate passed; mise.toml and hk.pkl verified byte-identical to HEAD after generation; full mise run check exits 0 with 148 Rust tests, 11 installer tests and 97.64% line coverage after the fixture repair and app-path move |
| Regeneration is repeatable | A second cospec init reports unchanged files and update check passes | Second init created no files; managed drift check passed; cospec doctor reported zero errors and zero warnings |
