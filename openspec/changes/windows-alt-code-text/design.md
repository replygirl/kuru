# Design

## Context

Original native receipts expose U+0301 as Char/Release/NONE after raw UTF-8 input; ordinary characters have Press/Release pairs. Crossterm PR745 discusses the Alt-code event intentionally. Ratatui and Helix recommend ignoring Release, which Kuru already does. Public events do not retain enough native identity for a safe missing-press heuristic.

## Decisions

Keep latest released registry crossterm 0.29.0, unchanged production input policy, and the complete Kuru coverage inventory. Remove the rejected dependency source and all related exceptions.

The test terminal is a ConPTY host. Microsoft documents its startup `CSI ?9001h` request and `CSI Vk;Sc;Uc;Kd;Cs;Rc_` native keyboard records. Observe that request through actual bounded output; never silently fall back. Add small fixture helpers for committed UTF-16 character units and explicit Enter/Escape/Backspace/arrows/Ctrl-R/Ctrl-E, retaining the existing writer and process ownership. The recall scenario uses these helpers consistently, avoiding ambiguous bare Escape after the native protocol activates. Keep other raw-byte fixture scenarios unchanged.

## Operational surface

Existing native ConPTY, public EventStream/renderer and owned cleanup run on x64 and ARM. No production backend, provider, endpoint or topology changes; deadlines remain unchanged.

## Integration contract

Require exactly one U+0301 Press and one Release, exact filtered draft bytes, unchanged ordinary release exclusion, and seven exact completed persisted prompts at 120/80 columns. Native e6f4ee77 x64 input probe passes, while both recall tests fail the old exact-output projection predicate (18-character draft reported, missing space/cat in ConPTY text). Check exact Kuru TestBackend glyph cells and cursor first. Synchronize the native decomposed draft on its stable ASCII composer anchor and visible cursor at the full canonical display width, retaining labels, initial character count at wide size, exact durable bytes and bounded cursor/output diagnostics. Do not enumerate more lossy glyph strings or claim full ConPTY glyph fidelity; no own renderer defect is confirmed. Assertions follow native cleanup. Keyboard framing demonstrates committed-key behavior only: Windows Terminal paste intentionally sends raw text, so this does not fix or verify pasted decomposed text or genuine Alt-code commitment. Existing atomic Paste portability limitations remain explicit.

## Risks / Trade-offs

Actual native execution must prove protocol recognition and command behavior; cross-target lint is not native evidence. Supplementary characters use UTF-16 units as the documented wire requires. No key-pair/focus heuristic is introduced.

## Sources

- https://github.com/crossterm-rs/crossterm/pull/745
- https://ratatui.rs/faq/#why-am-i-getting-duplicate-key-events-on-windows
- https://github.com/helix-editor/helix/pull/6139
- https://github.com/rhysd/tui-textarea/pull/17
- https://github.com/microsoft/terminal/blob/main/doc/specs/%234999%20-%20Improved%20keyboard%20handling%20in%20Conpty.md
- https://github.com/microsoft/terminal/blob/main/src/cascadia/TerminalCore/Terminal.cpp
- https://github.com/microsoft/terminal/blob/main/src/cascadia/TerminalControl/ControlCore.cpp
- https://docs.rs/crate/crossterm/latest

- https://github.com/microsoft/terminal/pull/16916
- https://github.com/microsoft/terminal/wiki/Console%3A-Potential-Breaking-Changes
