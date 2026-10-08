# Tasks

## 1. Scope correction and causal research

- [x] 1.1 Remove all rejected vendored source, root Cargo patch, coverage filter, extra development dependencies, formatter exception and vendor conventions; restore normal latest-release registry resolution and unchanged Kuru source inventory/95% gates.
- [x] 1.2 Research primary crossterm/ratatui/popular-dependent sources and Microsoft's ConPTY protocol; distinguish committed-key fixture correction from unresolved raw-text paste/Alt-code fidelity.

## 2. Kuru acceptance

- [x] 2.1 Implement the documented native-key fixture protocol with observed negotiation, exact decomposed text receipts and explicit command keys; qualify native keyboard acceptance separately from paste/Alt-code limitations.
- [ ] 2.2 Run relevant static checks and independent review, then observe exact input and seven-prompt recall acceptance at 120/80 columns on Windows x64 and ARM before archive.

Observed locally: Windows-target TUI lint, formatting and docs checks pass. Independent implementation review clears negotiated BMP-only text, explicit command framing, release exclusion and cleanup. Native task2.2 remains pending.
