# Spec Delta

## ADDED Requirements

### Requirement: Plain memory-open activity

While a command opens project memory, the application SHALL show one plain sentence naming Kuru's current activity, from a fixed set of five, without a title, label, stage name, jargon, elapsed counter or estimate. The five sentences are `Opening this project's memory…`, `Getting Kuru's memory ready on this computer…`, `Creating this project's memory…`, `Upgrading this project's memory…` and `Waiting for another copy of Kuru that is using this project's memory…`; each SHALL end with U+2026, be defined once as one constant in one module, and appear with no `Memory:` prefix and no ready line. A sentence other than the opening sentence SHALL appear only after an observed stage begins that activity, and SHALL be replaced when a later observed stage begins different work; stages that begin no new activity, and unknown stages, SHALL NOT change it. A wait shorter than one 100 ms tick MAY never be shown. On a terminal the sentence SHALL be rewritten in place, shortened to the terminal width while keeping its trailing ellipsis, measured so that ambiguous-width characters cannot cause wrapping or an incomplete erase, and erased when memory is ready or the open fails; otherwise each new sentence SHALL be written once on its own line, never repeating the previous line, and nothing SHALL be written at ready. The sentence SHALL be written to standard error, or, for an interactive session whose standard error is not a terminal, to the interactive terminal before the interface starts; it MUST NOT reach standard output, and `--json` output MUST be unchanged. An output failure SHALL stop further sentences without affecting the open. The notices about leftover engine setup files SHALL follow a successful open on their own line and name the configured cache folder when `memory.cache_dir` is set.

#### Scenario: First launch on a terminal
- **WHEN** interactive Kuru first opens a new project's memory on a terminal
- **THEN** the terminal shows the opening sentence and then the project-creation sentence, and erases the line before the interface starts.

#### Scenario: Headless JSON
- **WHEN** `kuru run --json` first opens a new project with an empty engine cache and a non-terminal standard error
- **THEN** standard output is exactly the parseable result
- **AND** standard error holds the opening sentence first, then the engine-preparation sentence before the project-creation sentence, each of those two once, every line one of the fixed sentences, no line equal to the line before it, and no ready line.

#### Scenario: Reopening
- **WHEN** an existing current project is opened again and no other process holds it
- **THEN** only the opening sentence appears.

#### Scenario: Another process holds the project
- **WHEN** another process holds owner authority without an attachable endpoint
- **THEN** the waiting sentence appears
- **AND** the opening sentence replaces it once that process releases authority and Kuru starts its own owner.

#### Scenario: Upgrade
- **WHEN** an existing project's stored memory is upgraded to this version's format
- **THEN** the upgrade sentence appears, and an upgrade that is part of creating a project keeps the creation sentence.

#### Scenario: Interactive with redirected standard error
- **WHEN** interactive Kuru starts with standard error redirected to a file
- **THEN** the sentence appears on the terminal before the interface, and none is written to that file.

#### Scenario: Narrow terminal
- **WHEN** the terminal is narrower than the current sentence
- **THEN** the sentence is shortened to fit on one line and still ends with an ellipsis.

#### Scenario: Open fails
- **WHEN** the memory open fails on a terminal
- **THEN** the sentence is erased before the error is printed.

### Requirement: Open marker lines

Only when the environment variable `KURU_OPEN_MARKERS` is exactly `1`, the release binary SHALL write to standard error, whether or not standard error is a terminal, one ASCII line per event, flushed after each line and carrying nothing else: `kuru-open-marker v1 <event> <monotonic_ns>`. The events SHALL be `open-start`, before the client's first attach attempt; `waiting-ownership`, once and only when a project-ownership wait is shown; and `ready`, where memory becomes ready, including an attach to a running owner, and not on failure. `<monotonic_ns>` SHALL be an unsigned decimal taken from one monotonic clock anchor per process. On a terminal a marker line MUST NOT corrupt the in-place sentence: the sentence is erased, the marker written, and the sentence redrawn. A marker write failure MUST NOT stop sentences or affect the open. With the variable unset, nothing SHALL change. The variable MUST be documented only in the developer documentation.

#### Scenario: Markers off
- **WHEN** `KURU_OPEN_MARKERS` is unset or not `1`
- **THEN** no `kuru-open-marker` byte is written anywhere.

#### Scenario: Markers on, uncontended
- **WHEN** `KURU_OPEN_MARKERS=1` and a command opens a new project with a non-terminal standard error
- **THEN** `open-start` is the first standard error line, `ready` appears once after every sentence, no `waiting-ownership` appears, non-decreasing nanosecond values are used, and standard output is unchanged.

#### Scenario: Markers on a terminal
- **WHEN** `KURU_OPEN_MARKERS=1` and the sentence and markers share a terminal
- **THEN** no marker shares a line with a sentence, and the final visible line after ready is empty.

#### Scenario: Marker while waiting
- **WHEN** a project-ownership wait is shown
- **THEN** exactly one `waiting-ownership` marker is written for that open.
