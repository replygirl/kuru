## 1. Semantic event projection [critical]

- [x] 1.1 @regression (agent) emit secret-bearing event metadata and detail -> focused controls passed for text error plus structured state, peer and relationship data; the actual-Dolt broadcast/output/journal/exact-replay control found no raw recognizable secret in actor or detail.
- [x] 1.2 @unit (agent) project structured state/peer values and malformed legacy detail -> safe activation, peer body and relationship member fields remained; malformed structured legacy data produced only `[event detail withheld]`.
- [x] 1.3 @regression (agent) complete and exactly retry a turn -> runtime and CLI/TUI controls passed with `[response completed]` in events, the answer retained only in `TurnOutput.text`, and the exact completed result reused without new work.
- [x] 1.4 @eval (agent) inspect recording-provider and serialized semantic-event fixtures -> projected output retained nonsecret state/peer meaning, provider prompts remained covered by the 86-test runtime suite, and actor/detail secret fixtures were absent from broadcast, output, journal and replay serializations.

## 2. Honest outcomes and compatibility

- [x] 2.1 @unit (agent) independently exhaust tool calls and peer rounds and produce empty final text -> focused actual-Dolt controls passed with only `tool-calls`, only `peer-rounds`, and a separate `empty` response outcome respectively.
- [x] 2.2 @unit (agent) deserialize a v0.3.2 `limited: true` fixture -> the compatibility control replayed it as `legacy-unspecified` without choosing a concrete budget; TUI legacy and empty rendering controls passed.

## 3. Exact local retry and interruption [critical]

- [x] 3.1 @integration (agent) retry a completed local turn through runtime and CLI/TUI adapters -> runtime, CLI and TUI controls passed with unchanged provider counts and durable/displayed transcript rows; A2A traffic did not replace the local tuple.
- [x] 3.2 @regression (agent) retry a possibly dispatched interruption -> runtime and real-PTY controls refused it before new provider/tool work; the existing file-mutation control also left its target absent.
- [x] 3.3 @integration (agent) resume after interruption and after a later successful turn -> runtime and real-PTY process-resume controls retained one fixed marker, excluded it from provider context, preserved the later answer and reused the completed last submission without adding rows.
- [x] 3.4 @regression (agent) race cancellation with the ended checkpoint -> `accepted_ended_checkpoint_wins_cancellation_and_publication_error` and the TUI completion adapter control passed with the committed answer and no false marker.

## 4. Diagnostics and retention

- [x] 4.1 @integration (agent) run JSON mode with `--debug` -> the focused CLI control parsed unchanged stdout, matched stderr's resolved checked directory, and verified all four bounded rotated slots at that path.
- [x] 4.2 @integration (agent) complete multiple identified turns in actual Dolt and retry earlier IDs -> runtime/CLI controls retained each journal and completed output, reused the selected earlier result and added no transcript rows; implementation adds no expiry, cap, TTL or pruning.
- [x] 4.3 @manual (agent) review runtime/session/command documentation -> reviewed architecture, usage, interface and docs-site pages distinguish the four-file 64 KiB operational ring from durable proportional no-expiry journal growth and intentional completed-output retention.

## 5. Static checks

- [x] 5.1 @integration (agent) run focused runtime/TUI tests, PTY checks, docs checks, format, lint and typecheck -> runtime task passed 86 tests; runtime/TUI lint and typechecks, Rust format, TUI focused live and pure controls, real PTY capture, and docs build/format/content/link checks passed.
