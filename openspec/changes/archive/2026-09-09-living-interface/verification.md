## 1. Persistent choices [critical]

- [x] 1.1 @e2e (agent) Actual binary selector changes, quit/relaunch -> four PTY launches in `preferences.rs` pass: slash commands, F2/F3/F4, restored mode/model/effort, default clearing and zero-turn fresh sessions.
- [x] 1.2 @integration (agent) Provider pairs, precedence, project isolation, resumption, failed writes -> core/config, runtime preferences and four CLI preference tests pass. Real SQLite final-write rejection rolls back saved and live mode.

## 2. Complete visual identity [critical]

- [x] 2.1 @manual (agent) Wide/narrow scene, selector, conversation/error inspection -> actual colored cell exports for four 140×50 portraits, 70×25 compact scenes, model/mode pickers, relationship conversation and provider error reviewed through cmux browser screenshots. Simplified duplicated welcome status after review.
- [x] 2.2 @e2e (agent) Typing, paste, selectors, cancellation, resize and restoration -> real PTY smoke and delayed local HTTP provider tests pass. Cancellation retains draft, displays status, ignores late response and permits a successful next turn.
- [x] 2.3 @integration (agent) Combined controls and truthful activity -> nine visual tests plus four scene tests pass. Current value/key pairs stay together, long future labels retain shortcuts, previews do not mutate live mode, real routes/member names remain visible, private message content remains absent.

## 3. Responsive ambient motion

- [x] 3.1 @integration (agent) Ambient frames, decaying input and static override -> supplied-clock and buffer tests plus real PTY verify continuous ambient after four seconds, 250ms idle / 80ms interaction ceilings, 1.1s input decay, focus pause, static ornament and stable draft/caret.
- [x] 3.2 @benchmark (agent) Frame costs -> explicit `frame_cost_profile` passed: 200 frames each at 140×50, debug build; welcome 1.212ms, 500-entry conversation 2.256ms, open mode picker 2.726ms per frame. No timing threshold in CI.

## 4. Delivery

- [x] 4.1 @regression (agent) Full `mise run check` -> exit 0; 145 Rust tests, 11 installer tests, 97.63% workspace coverage (6797/6962), lint/format/tooling/spec/drift checks all pass. The explicit benchmark also passes separately.
- [x] 4.2 @runtime (agent) Final cmux build -> existing `Kuru · new design` surface relaunched with the same `/tmp/kuru-vivid-preview-20260909` state store; combined `gpt-6-astra F2`, `medium F3`, framework F4 controls observed. Actual preference restart proof is recorded in 1.1.
- [x] 4.3 @integration (agent) Artifacts and managed drift -> full gate strict validation and `cospec update --check` pass with no drift; completed acceptance ledger prepared for final strict validation and archival. Updated interface, usage, configuration and verification docs.
