# Verification

## 1. Themes and capability negotiation [critical]

- [x] 1.1 @integration (agent) exercise native parser and published schema with valid/invalid UI settings, explicit layer precedence and managed constraints -> both contracts agree and preserve provenance/constraints
- [x] 1.2 @e2e (agent) drive completed actual PTY frames in truecolor, 256-color, classic 16-color and NO_COLOR modes -> emitted sequences stay in the advertised depth and NO_COLOR removes decoration without removing meaning
- [x] 1.3 @integration (agent) exercise the pinned backend's real classic color encoding and any required streaming conversion -> classic output is correct across fragmented/combined writes and unrelated output passes unchanged

## 2. Current presentation remains usable [critical]

- [x] 2.1 @integration (agent) render current permission/error/status, composer, cards and scene frames at 80 and 120 columns with dark/light/plain/limited palettes and Unicode content -> text, controls, selected choices and cursor geometry remain legible; shipped semantic roles retain contrast
- [x] 2.2 @e2e (agent) exercise human CLI output under capable/NO_COLOR/redirected streams and structured JSON -> only capable human output may be colored and structured/redirected bytes remain plain
- [x] 2.3 @integration (agent) run owning host/Windows static checks, docs build/content checks, format and managed checks -> observed relevant checks pass, with native CI limitations explicitly recorded

## Observed local evidence

- Owning core test (`mise run //packages/kuru-core:test -- -- theme_configuration_has_schema_parity_layered_leaves_and_no_authority_claim --exact --nocapture`), handle 47021: terminal exit 0, one selected test passed. Native and published/managed schemas agree; palette leaves preserve layered values and final source provenance, typed overrides cannot bypass managed constraints, and UI preferences do not alter workspace authority.
- Corrected owning app selection (`mise run //apps/kuru-tui:test -- -- theme ansi16 --nocapture`, with the package-owned prefetch and isolated Dolt cache), handle 38971: terminal exit 0, ten selected unit tests and one actual PTY test passed; unrelated suites were filtered. Pure checks cover negotiation, shipped role distinctions, bounded conversion, short writes/errors, RGB parameter groups, permission/error/card/scene text and Unicode cursor geometry at 80/120 columns. A direct scene draw retains its own palette after another surface renders.
- The actual PTY fixture observes completed frames, the unchanged default dark accent, light/override RGB, indexed 256, classic 16, limited-terminal plain and NO_COLOR output at practical widths. It verifies ordinary human MCP warnings on capable/NO_COLOR terminals, plain redirected catalog JSON and headless JSON, and normal terminal restoration. The MCP entry is disabled; no configured process or live provider inference runs.
- First app selection 34120 ended 101: eight of nine units passed and the PTY completed all six depth cases before two assertion errors. The backend unit inherited the runner's NO_COLOR and correctly emitted empty SGR; its expectation now acknowledges actual suppression without changing global environment. The human CLI assertion used the final screen after long JSON scrolled the warning away; the corrected assertion uses retained output bytes. These were test assumptions, not claimed prior passes.
- Root source review required removing ambient palette state. All ACTIVE palette state and accessors were removed; pure helpers receive the owning Theme, and existing View arguments supply it directly. Initial incomplete wiring failed compiler check 1753; corrected check 59519 ended 0. Its final test-source edit prevented cache storage, so frozen selection 38971 supplies the final complete compilation and behavior evidence.
- Owning host and Windows app lint, Rust format, public docs build/format/lint/content/artifact/link/anchor checks: graph 3834 terminal exit 0. Earlier graph 38233 ended 101 on a derivable Default implementation, corrected using derive; its cancelled Windows task was not counted as a pass. Core owning lint passed in that graph. Owning platform host/Windows lint and managed drift graph 39537 ended 0. Core/app compiler checks include all targets/features.
- Public documentation describes finite configuration, precedence, color-depth negotiation, NO_COLOR, exact-stream human status eligibility and unchanged structured/redirected output. No tool versions, dependencies, pins, managed files or task definitions changed.

## Proof limits

Local native behavior was exercised on macOS with isolated demo memory and actual PTYs. Native Windows console fixture assertions are authored and cross-target checked; native Windows execution and Linux behavior remain the existing CI responsibilities. Broad suites, combined coverage and remote CI were not rerun locally; the unchanged coverage gate and delivery workflow apply to the branch. No real user memory or credential files were inspected, and no paid inference, browser login, installation or publication was performed by this change.
