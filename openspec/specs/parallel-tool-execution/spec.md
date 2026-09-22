# parallel-tool-execution Specification

## Purpose
Define how Kuru admits independent native reads in provider order, executes them concurrently within runtime and retained-handle bounds, preserves each call's checked authority and identity, drains cancellation safely, and returns deterministic protocol results while reporting truthful completion activity.

## Requirements

### Requirement: Ordered admission and bounded independent execution

For a provider completion with several tool calls, the runtime SHALL admit each call in provider order against the remaining turn budget and the connector's actual effect classification. It SHALL execute contiguous already-authorized independent native reads in bounded concurrent waves, including `file_read`, `file_list`, `grep`, `glob` and `web_fetch` when their exact per-call authority can be held through execution. A fresh foreground or request-bound Once approval SHALL dispatch its call immediately through the serial path. Mutating, cognitive, A2A, skill-activation, shell, MCP and unclassified calls MUST be serial barriers unless a later separately specified contract proves independence. No model-provided name, argument or annotation may declare its own call independent.

#### Scenario: Native read wave overlaps under a bound

- **WHEN** a completion contains two already-authorized independent native reads and the configured concurrency limit permits two
- **THEN** both enter checked execution before either settles, peak active calls remain within the limit, and neither waits for the other's result

#### Scenario: Fresh approval and mutation remain serial

- **WHEN** an eligible read requires a fresh Once answer or a wave is followed by a file mutation, shell, MCP, cognitive or unknown call
- **THEN** the Once call dispatches under its original request-bound approval before later admission, and the serial call starts only after every earlier wave call has settled or drained

### Requirement: Per-call authority, cancellation and receipts

Each call SHALL retain its validated arguments, provider call ID, admitted turn/invocation identity, own permission result, cancellation fence, and result receipt. Search admission MUST discover bounded checked candidates, decide each candidate's exact permission, and review only allowed candidates' nested instruction ancestry before any path or content is exposed. Parallel checked execution MUST revalidate the root, checked target or candidate facts, current permission authority and instruction generation before an effect or result; a changed fact SHALL refuse or require replan rather than reuse stale authority. `web_fetch` SHALL retain its existing per-connection and per-redirect destination validation. A denial or required headless review MUST dispatch zero effects for that call. Cancellation SHALL stop admission, cancel and drain owned wave work before return, emit one settled observation per admitted call, and MUST NOT replay an ambiguous effect.

#### Scenario: Search review precedes overlap

- **WHEN** `grep` and `glob` have checked allowed and denied candidates without a fresh foreground decision or instruction activation
- **THEN** their bounded candidate admission is serial, denied paths never enter result or instruction review, and the allowed checked search executions can overlap other admitted reads

#### Scenario: Authority changes after admission

- **WHEN** an admitted path's checked permission facts, workspace, permission grant or nested instruction generation changes before checked execution
- **THEN** that call returns a bounded refusal or replan result without reading the changed target or publishing unreviewed instruction bytes

#### Scenario: Cancellation during an active wave

- **WHEN** a controlled turn is cancelled with native reads or a bounded fetch active
- **THEN** the runtime drains the owned calls, settles each admitted call once with its actual outcome, and performs no later serial mutation or provider continuation

### Requirement: Deterministic continuation and truthful activity

The runtime SHALL preserve the provider's call IDs, item/call distinction, per-call budget accounting and original call order in the protocol-complete result continuation, even when execution and projected progress settle out of order. An instruction publication SHALL create a replan boundary for later calls rather than allowing them to run under stale prompt authority. Start and settlement events SHALL describe the actual active calls, and the TUI SHALL remove each settled call from its activity set without removing another still-running call.

#### Scenario: Out-of-order settlement, ordered continuation

- **WHEN** the second admitted read settles before the first
- **THEN** projected activity reflects the second settlement immediately, while the provider receives one bounded result for every call in original order with unchanged call IDs

#### Scenario: Instruction activation stops later dispatch

- **WHEN** a read activates newly reviewed path-specific instructions during ordered admission
- **THEN** no later call in that completion executes under the previous instruction state and its result explains the required replan
