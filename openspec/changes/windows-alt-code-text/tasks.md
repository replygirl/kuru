# Tasks

## 1. Exact dependency correction

- [x] 1.1 Verify and retain the exact crossterm 0.29.0 upstream archive, MIT license and provenance in the TUI-owned vendor directory; apply one root Cargo patch with unchanged dependency version and application workspace inventory. Exact archive SHA matches; all77 files retained, only parser source differs after LF normalization. Offline locked metadata resolves one exact0.29.0 dependency and8 unchanged members; independent review clears.
- [x] 1.2 Correct the Windows Alt-code parser branch to emit committed text as Press while retaining ordinary Release events, with no Kuru UI heuristic or unsafe exemption. Exact discriminator patch reviewed; Windows-target TUI lint passes. Corrected native acceptance remains task2.1.

- [x] 1.3 Preserve the foreign-dependency reporting boundary with one exact versioned directory filter in canonical exports and local workspace coverage; verify Unix/Windows boundaries and retention of every application source. Delivery orchestrator tests28/28 pass, including all export arguments, OS boundaries and every Kuru source module. Independent review clears; pinned LLVM accepts the regex against saved profiles (syntax only, no new coverage claim).

## 2. Causal acceptance

- [ ] 2.1 Preserve the original native regression's exact decomposed Unicode input and seven persisted prompts; require one accent commit and distinct ordinary press/release events. Observe the original native failure and corrected native pass.
- [x] 2.2 Update contributor/vendor documentation, run relevant static and documentation checks, independently review the exact patch/provenance and name native checks honestly until CI executes them. Format, Windows-target TUI lint, delivery host lint and docs checks pass; exact source/provenance and coverage-boundary reviews clear. Native acceptance remains pending in task2.1.
