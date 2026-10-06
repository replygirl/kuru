# Proposal

## Why

The Phase 2 U6/P19 outcome requires useful one-shot scripting with piped input, public streaming output and honest exit status. Current Run requires an argument and prints its final result synchronously before cleanup, so a held stdout reader can prevent cancellation and release of owned work.

## What Changes

- Explicit Run accepts optional argv plus bounded UTF-8 redirected stdin, composing both with a literal-data label before configuration, trust or runtime authority activates. Bare Kuru keeps its terminal requirement.
- Add mutually exclusive `--output-format stream-json` alongside the unchanged final `--json` shape, with versioned public status, facing-text snapshot, gap and terminal records.
- Use one bounded stdout-only OS writer with acknowledgements while the same controlled turn and cancellation remain independently polled; cancellation or closed output awaits existing authority cleanup before exits 130/141.
- Carry the original cancellation token through controlled exit shutdown, retaining the existing shutdown wrapper, dream policy, 30-second budget and awaited cleanup.
- Introduce a narrow typed exact-request mismatch rejection for exit 2; preserve overall typed denial, completed result, unresolved failure, exact retry and private-content boundaries.
- Document stdout/stderr, input precedence, finite public grammar, gaps, blocked output and exit semantics; verify real subprocess and HTTP interactions with owned cleanup.

## Capabilities

### New Capabilities

- `headless-run-stream`: bounded one-shot stdin admission, public JSONL records, exit status and output-independent controlled cleanup.

### Modified Capabilities

None. Existing final TurnOutput, Event serialization, journal, provider routes and operational diagnostic contracts remain unchanged.

## Impact

CLI Run fields, its effect-free input boundary and controlled output handling; a small headless module and binary typed-exit adapter; a typed runtime exact-request rejection; existing terminal/CLI fixtures and public command/first-conversation documentation. No database, receipt, journal, wire protocol, dependencies, task workspace or migration changes. No batch loop, control API, raw event/detail/tool-result stream, private reasoning/peer payloads or notification framework.

## Surfaces

- [x] interactive — explicit CLI input, output and exit UX
- [ ] deploy — unchanged runtime topology and installation
- [x] integration — versioned machine-output contract
- [ ] agent-behavior — existing prompts, tools and routes retained
