## 1. Instruction authority and exact-byte injection [critical]

- [x] 1.1 @regression (agent) approve a real workspace, then add, change, remove and replace automatic `AGENTS.md` sources -> `automatic_instruction_sources_are_ordered_safe_and_stale_complete_approval` passed on macOS; changed, removed, restored and same-byte native replacement sources each made status report a non-matching complete approval
- [x] 1.2 @integration (agent) snapshot ordered outer and project-root instructions, mutate the files, and construct the runtime with a recording provider -> `runtime_injects_only_the_instruction_bytes_owned_by_the_reviewed_snapshot` passed against real temporary Dolt; every recorded request contained outer then root reviewed bytes and excluded both later file contents
- [x] 1.3 @regression (agent) invoke an unapproved runtime command with an automatic instruction and a provider side-effect fixture -> the automatic-source regression passed with an unreadable explicit Responses key; preflight reported only the instruction claim before provider environment decoding and created no private data
- [x] 1.4 @eval (agent) inspect a recording provider's actual system prompts for a multi-source snapshot -> the same real-runtime test inspected every DemoProvider request and found the two reviewed sources in precedence order with no unreviewed marker

## 2. Review surface and command subsets

- [x] 2.1 @integration (agent) inspect status for multiple automatic sources including the root -> the automatic-source regression showed `project instructions: 2 ordered automatic sources`, listed bounded outer then root paths and excluded both instruction bodies
- [x] 2.2 @regression (agent) exercise the documented CLI command matrix with automatic instructions present -> `real_cli_commands_enforce_the_documented_claim_matrix_before_side_effects` passed all runtime, tool, model, memory, auth and inspection cases without spawning configured transports or creating state before approval

## 3. Static and documentation checks

- [x] 3.1 @unit (agent) run targeted `kuru-core` configuration tests -> `mise run //packages/kuru-core:test` passed all 33 unit/integration tests, including ordered root capture, aggregate bounds, frozen bytes and same-byte native replacement identity
- [x] 3.2 @unit (agent) run targeted `kuru-tui` trust tests and applicable format/lint tasks -> three selected trust/runtime matrix tests passed; `mise run //packages/kuru-core:lint`, `mise run //apps/kuru-tui:lint`, Rust format check and docs format check passed
- [x] 3.3 @manual (agent) inspect `docs/configuration.md` against the implemented claim and command matrix -> reviewed text states project-root and home-ancestor inclusion, complete invalidation, runtime-only consumption, ordered safe output and exact snapshot-byte injection without defining global instructions
