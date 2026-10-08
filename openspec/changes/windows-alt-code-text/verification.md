# Verification

## 1. Exact native committed-key input [critical]

- [ ] 1.1 @regression (agent) same owned ConPTY and public EventStream, documented native committed-key transport -> exactly one U+0301 Press and Release; filtered bytes equal the original decomposed draft. Original raw-text CI37853387922 demonstrated Release-only loss on x64 and ARM; it is not evidence that all ordinary keyboard input loses the accent.
- [ ] 1.2 @integration (agent) actual mode negotiation and explicit controls -> no silent raw fallback, duplicate ordinary characters, release-triggered commands or ambiguous bare Escape after native protocol activation.
- [ ] 1.3 @e2e (agent) public renderer/native recall -> all seven completed synthetic prompts remain exact at120/80 columns on Windows x64 and ARM; restore terminal modes and owned processes before comparison.

## 2. Repository and dependency boundaries

- [x] 2.1 @integration (agent) source/config inspection -> rejected vendor/source patch and reporting/formatter conventions removed; Cargo.lock restores registry crossterm0.29.0. Docs.rs latest and upstream release list confirm0.29.0; all Kuru source and95% gates remain.
- [ ] 2.2 @integration (agent) relevant static checks, independent review and acceptance documentation -> targeted fixture-only correction; raw-text paste/Alt-code and atomic Paste limitations remain explicit. Native passes recorded only after actual execution.
