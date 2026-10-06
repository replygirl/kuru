# Spec Delta

## ADDED Requirements

### Requirement: Effect-free bounded one-shot input

Explicit `kuru run` SHALL accept optional argv and redirected UTF-8 stdin once, validate the aggregate 1–131072-byte nonblank prompt before any workspace/configuration/provider/tool/memory authority activates, preserve argv bytes for an empty pipe and compose nonempty argv plus pipe with a documented literal-data label. TTY stdin MUST NOT be read. Bare Kuru MUST retain its interactive terminal requirement. Invalid UTF-8, over-bound or empty input MUST reject before authority; cancellation while waiting for stdin MUST exit 130 only at that effect-free boundary.

#### Scenario: Argument and pipe precedence
- **WHEN** explicit Run receives argv, pipe-only input, argv plus nonempty pipe, an empty pipe or terminal stdin
- **THEN** exactly one bounded prompt follows documented precedence, terminal input is not read and the exact composed request is the existing retry identity input.

#### Scenario: Rejected or interrupted input
- **WHEN** input is malformed, empty, over-bound or interrupted before EOF
- **THEN** no configured authority activates and rejection uses exit 2 or interruption uses exit 130.

### Requirement: Versioned allowlisted public stream

Run SHALL offer mutually exclusive `--output-format stream-json` and existing final `--json`. Stream JSONL v1 MUST use checked monotonic record sequence and fixed started/status/snapshot/gap/terminal kinds, a safe exact turn correlation and bounded redacted/control-safe public fields. Facing progress MUST be labeled a replaceable snapshot with exact turn/request-round and actual snapshot sequence/truncation; unknown watch loss MUST remain distinct from exact broadcast omissions. It MUST NOT serialize raw Event/detail, summaries, histories, tool arguments/results, cognitive/peer bodies, credentials or private actor output. Replay MUST NOT invent missing live identity/order. Existing final JSON MUST preserve TurnOutput shape; debug, compaction, hook and error output MUST remain on stderr.

#### Scenario: Partial progress, lag and round changes
- **WHEN** public activity or facing text advances faster than a consumer, or changes request round
- **THEN** bounded snapshots and explicit gap semantics retain correct turn/round without claiming exact token-loss counts or exposing excluded private payloads.

#### Scenario: Completed and reused result
- **WHEN** one turn completes or an exact completed request is retried
- **THEN** a writable stream receives one terminal record with honest bounded answer/truncation and reused metadata, final JSON remains compatible, and retry causes no provider/tool replay.

### Requirement: Output-independent owned settlement

Stdout writing MUST NOT prevent polling the controlled turn or cancellation. One bounded stdout-only writer MAY remain blocked until process termination but MUST hold no runtime authority, and interruption MUST await the same admitted operation plus existing harness, memory, diagnostic and lease cleanup before exit. Broken stdout SHALL exit 141; SIGINT SHALL exit 130 without rewriting an already completed journal. Success SHALL await acknowledged output. No-effect rejection SHALL exit 2, exact overall typed denial 3 and other unresolved/cleanup failure 1; nonfatal denied tools MAY complete successfully with exit 0. Exactly one terminal SHALL be attempted only when writable, and documentation MUST describe absent/partial terminal on broken or held output without promising replay.

#### Scenario: Held reader and SIGINT
- **WHEN** an admitted effect or completed answer is followed by a stdout reader that stops draining and SIGINT
- **THEN** cancellation/settlement and owned cleanup proceed independently, exit 130 preserves any accepted completion and exact retry returns that completion without replay.

#### Scenario: Interrupted default exit dream
- **WHEN** the completed Run starts its configured exit dream and SIGINT arrives during held inference
- **THEN** the original token cancels that same controlled shutdown, all ownership cleanup is awaited before exit 130 and the completed turn journal remains unchanged; already-cancelled tokens do not start a new exit dream.

#### Scenario: Broken output after effect
- **WHEN** stdout closes after admitted work
- **THEN** the same operation settles, owned cleanup completes before exit 141 and no ambiguous tool/provider work is replayed.

#### Scenario: Typed refusal and provider failure
- **WHEN** input mismatch, overall permission denial, provider failure or cleanup uncertainty occurs
- **THEN** public status and exits follow exact typed evidence, raw error bodies stay out of JSONL and no substring classifier fabricates a no-effect result.
