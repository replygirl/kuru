# Verification

## 1. Exact native committed-key input [critical]

- [x] 1.1 @regression (agent) same owned ConPTY and public EventStream, documented native committed-key transport -> exactly one U+0301 Press and Release; filtered bytes equal the original decomposed draft. Original raw-text CI37853387922 demonstrated Release-only loss on x64 and ARM; it is not evidence that all ordinary keyboard input loses the accent.
- [x] 1.2 @integration (agent) actual mode negotiation and explicit controls -> no silent raw fallback, duplicate ordinary characters, release-triggered commands or ambiguous bare Escape after native protocol activation.
- [x] 1.3 @e2e (agent) public renderer/native recall -> all seven completed synthetic prompts remain exact at120/80 columns on Windows x64 and ARM; restore terminal modes and owned processes before comparison. Exact TestBackend cells/cursor must retain combining grapheme, following space and wide glyph; native synchronization must query the actual console through released crossterm and require the full canonical cursor with native/projected diagnostic coordinates, without claiming native glyph fidelity.

## 2. Repository and dependency boundaries

- [x] 2.1 @integration (agent) source/config inspection -> rejected vendor/source patch and reporting/formatter conventions removed; Cargo.lock restores registry crossterm0.29.0. Docs.rs latest and upstream release list confirm0.29.0; all Kuru source and95% gates remain.
- [x] 2.2 @integration (agent) relevant static checks, independent review and acceptance documentation -> targeted fixture-only correction; raw-text paste/Alt-code and atomic Paste limitations remain explicit. Native passes recorded only after actual execution.

Completed76a076ea CI37865819250: x64 job113612420376 and ARM113612521869 pass full recall, requiring the actual native canonical caret and all seven exact completed prompts at120/80. Current committed-key probes113612420342/113612521962 pass exact filtered draft, raw down/up pairs and U301 Press/Release. Local exact TestBackend glyph/cursor/submission regression, Windows-target lint, formatting and normal push hooks pass; independent implementation review is clear. Windows workspace coverage remains94.22%, below the separate95% goal; original Ubuntu stale-socket fixture failure is corrected separately. No full-goal pass is claimed.
