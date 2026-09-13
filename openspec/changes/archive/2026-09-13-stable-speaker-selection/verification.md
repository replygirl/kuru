## 1. Speaker behavior [critical]

- [x] 1.1 @regression (agent) exercise the production selector with equal activations across differing turn counts -> exact production-harness assertion exited 101 at baseline `604b88e`, with distinct speakers in `/private/tmp/kuru-speaker-baseline-regression.log`; after restoration it exited 0 in `/private/tmp/kuru-speaker-corrected-regression.log`.
- [x] 1.2 @runtime (agent) exercise prior-speaker ties, strictly higher activation, unavailable previous peer, caller target and active focus -> real-Dolt fixture asserted each identity and fixed reason, including a second unique-high activation, in `automatic_speaker_selection_is_stable_and_persists_after_dolt_reopen` (exit 0).
- [x] 1.3 @integration (agent) complete a real-Dolt-backed turn, shut down and reopen the store, resume and speak again -> real-Dolt fixture closed and reopened its explicit store and retained the completed speaker through a resumed tie (exit 0).
- [x] 1.4 @integration (agent) fail speaking before completion publication and load a legacy session record -> `failed_speaking_does_not_replace_completed_speaker` retained live and persisted prior identity; `legacy_session_without_continuity_value_deserializes` passed (both exit 0).
- [x] 1.5 @eval (agent) use deterministic provider fixtures through the real harness across unchanged ties and changed activations -> fixed fake provider exercised unchanged ties, changed activation, target, focus, and unavailable prior through the production harness (exit 0).

## 2. Quality and portability

- [x] 2.1 @manual (agent) inspect documentation and emitted selection metadata -> `apps/kuru-docs/concepts/sessions.md` documents the rule; `mise run //apps/kuru-docs:check` exited 0.
- [x] 2.2 @integration (agent) run package format, lint, typecheck, documentation and combined coverage checks -> the integrated normal hook passed all eight categories and 26,266/27,518 lines (95.4503%) at head `86f7d5695d6ba53f0e7aaa04ea691b79e9277030`; `/private/tmp/kuru-pr14-integrated-prepush-retry.log`.
- [x] 2.3 @runtime (agent) execute the runtime suite on supported native CI platforms -> PR14 CI run 34747712455 passed the Linux and macOS native jobs and the complete Windows instrumented runtime suite; `automatic_speaker_selection_is_stable_and_persists_after_dolt_reopen`, `equal_activation_keeps_the_same_speaker_across_completed_turns`, and `failed_speaking_does_not_replace_completed_speaker` passed on Windows, `/private/tmp/kuru-pr14-windows-ci-86f.log`.
