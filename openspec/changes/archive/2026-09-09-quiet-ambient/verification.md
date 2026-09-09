## 1. Quiet motion [critical]

- [x] 1.1 @regression (agent) Type, paste and edit at a fixed clock; compare scene and composer cells -> observed failure before fix (`typing disturbed the ifs portrait or composer decoration`); passes afterward across all four modes, including paste, backspace and delete.
- [x] 1.2 @regression (agent) Advance time and compare glyphs and color -> observed failure before fix (`ambient animation changed glyphs or positions in ifs`); passes afterward across all modes at 6/12/18/24 seconds. Color still changes; per-channel 250ms step is at most two levels.
- [x] 1.3 @integration (agent) Clock, focus, static mode and selector behavior -> supplied-clock tests prove typing/paste do not accelerate or reset ambient frames; busy cadence, focus pause, static override and existing mode-picker tests pass.

## 2. Delivery

- [x] 2.1 @manual (agent) Actual frames and cmux -> exported real terminal cells under `/tmp/kuru-quiet-visual`, inspected Freudian contour screenshot, then refreshed the idle existing `Kuru · new design` surface with the same state store. Temporary browser review tab closed.
- [x] 2.2 @regression (agent) Full repository gate and docs -> `mise run check` exits 0: 147 Rust tests, 11 installer tests, 97.64% coverage (6731/6894), lint/format/tooling/strict-spec/drift pass. Updated usage, interface and verification docs. Repaired PTY-fixture output-draining starvation exposed in the first full run.
