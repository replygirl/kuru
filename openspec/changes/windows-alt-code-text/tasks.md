# Tasks

## 1. Scope correction and causal research

- [x] 1.1 Remove all rejected vendored source, root Cargo patch, coverage filter, extra development dependencies, formatter exception and vendor conventions; restore normal latest-release registry resolution and unchanged Kuru source inventory/95% gates.
- [x] 1.2 Research primary crossterm/ratatui/popular-dependent sources and Microsoft's ConPTY protocol; distinguish committed-key fixture correction from unresolved raw-text paste/Alt-code fidelity.

## 2. Kuru acceptance

- [x] 2.1 Implement the documented native-key fixture protocol with observed negotiation, exact decomposed text receipts and explicit command keys; qualify native keyboard acceptance separately from paste/Alt-code limitations.
- [ ] 2.2 Run relevant static checks and independent review, then observe exact input and seven-prompt recall acceptance at 120/80 columns on Windows x64 and ARM before archive.

Observed locally: Windows-target TUI lint, formatting and docs checks pass. Independent implementation review clears negotiated BMP-only text, explicit command framing, release exclusion and cleanup. Native task2.2 remains pending.

Native e6f4ee77: x64 committed-key probe passes; x64/ARM recall stops at the initial frame assertion before durable comparison. Finish narrow canonical-cursor synchronization and exact own-renderer cell verification; no deadline or accepted-byte changes.

Exact TestBackend regression passes at120/80: e+U301 remains one glyph cell, following space and wide 猫 remain intact, caret22 and submitted21bytes match canonical input. Native projection now binds composer marker/ASCII anchor and the full canonical caret, with bounded raw-tail/cursor diagnostics. Native recall remains task2.2 pending.

Follow-up:77a04cdf projected cursor reports21 instead of canonical22. The existing fixture control channel now queries the active console through released crossterm's cursor API and requires canonical22 under the original single deadline. Windows-target lint and independent implementation review pass; full native acceptance remains pending.
