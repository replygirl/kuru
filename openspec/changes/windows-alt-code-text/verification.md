# Verification

## 1. Native text commitment [critical]

- [ ] 1.1 @regression (agent) send exact `literal draft e\u{301} 猫` bytes through actual ConPTY and public terminal/EventStream -> original CI37853387922 job113571954580 fails: raw U+0301 is Release-only, filtered text loses its two UTF-8 bytes; corrected receipt must preserve exact input and one accent Press.
- [ ] 1.2 @integration (agent) inspect actual ordinary character events and persisted recall -> ordinary Press/Release pairs remain, text does not duplicate, and all seven completed prompt strings remain exact at 120/80 columns on native Windows x64 and ARM.
- [ ] 1.3 @e2e (agent) use the actual public live renderer and native recall fixture -> completed frames and seven submitted synthetic prompts preserve the original decomposed draft, with all native terminal modes and owned processes restored before comparison.

## 2. Dependency and repository boundaries

- [x] 2.1 @integration (agent) verify vendored bytes against the pinned original archive and inspect Cargo resolution -> exact 0.29.0 version, upstream MIT license/checksum/VCS retained, only Alt-code behavior changes, all consumers use one patched dependency, and Kuru workspace members remain unchanged. Observed: archive SHA and all77 files independently checked; offline locked metadata resolves one dependency and8 original members.
- [x] 2.2 @integration (agent) run Rust format, Windows-target TUI lint, host checks and public documentation checks -> pass without tool upgrades, unsafe consumer exceptions or relaxed coverage/Unicode assertions. Observed: format, native-target lint, delivery host lint,28 orchestrator tests and public docs checks pass. Native corrected input checks in section1 remain unrun until CI executes them.
