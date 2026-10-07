# Verification

## 1. Structural scope and actual catalog behavior [critical]

- [x] 1.1 @regression (agent) run the existing structural closing guard -> before: official Windows PR250 job112551153300 fails at trust.rs1643 (closing.rs189); macOS112551153061 and Ubuntu112551153083 report the same exact guard. After: owning app task76980 exits0 and the unchanged guard passes1/1 in0.03s, without exemptions.
- [x] 1.2 @integration (agent) run the three existing explicit catalog cases -> task76980 passes3/3 in5.41s: real HTTP unknown capabilities/exact1 call, legacy sentinel/no memory and unapproved Responses refusal assertions unchanged; all other targets filtered0.
- [x] 1.3 @equivalence (agent) compare the frozen source body -> mechanical comparison removes only the closing wrapper/added indentation and proves every original body statement identical; all other source bytes unchanged. Root independently reviewed the exact diff clear before checks.

## 2. Scoped repository checks

- [x] 2.1 @integration (agent) run app host/Windows lint and typecheck, Rust formatting, docs and managed checks -> host42301, Windows40910, type41230, format45357 and docs75309 exit0; managed check exits0/no drift. Initial multi-task command78629 exits101 because later task names were forwarded to clippy; corrected separate task invocations above establish the results.
- [x] 2.2 @integration (agent) settle the proportional artifact gate and contexts -> preimplementation and final strict/apply exit0 with skip_specs:true and all four returned contexts read; actual archive/physical checks are the next required operations before commit.
- [~] 2.3 @runtime (agent) fresh final-head native CI and exact-main checks -> defer: remote delivery follows actual archive and normal commit/push; no local native Windows claim or unchanged CI retry.
