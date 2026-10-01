# Proposal

## Why

A first launch of a project takes about six seconds and an existing project about 0.6 s (release build, macOS arm64). Since the memory service owner became its own process the owner opens silently, so the only text a person sees is a fixed stage label written by the starting process (`Memory: waiting for project ownership…`, `Memory: ready.`). Those lines use jargon, name stages rather than activity, and announce an ownership wait on every open even when none happens.

The command line should instead say, in one plain sentence, what Kuru is doing at that moment, erase it when memory is ready, and never claim an activity that is not happening. The owner must publish what it is doing so the client can say it. The open-time harness (PR #133) also measured to the `Memory: ready.` line and needs a replacement signal, agreed as opt-in marker lines.

## What Changes

- The command line shows one of five fixed plain sentences while memory opens, with no title, label, jargon or elapsed counter. On a terminal it is rewritten in place and erased at ready; otherwise each new sentence is written once on its own line; command output never receives it, and it goes through standard output only for an interactive session whose standard error is redirected, before the interface opens; `--json` is unchanged. Every `Memory:` string and the `ready` line are removed.
- Three internal stages are added (`CreatingDatabase`, `UpgradingDatabase`, `StartingMemoryService`); `WaitingForProjectOwnership` is reported only when a lock is actually busy.
- The owner publishes a private activity record (`activity.json`) beside `endpoint.json`, tagged with a SHA-256 derived value of the starter token that unit 1 already passes; the client forwards its new stages between readiness polls. The record carries no authority, never fails or slows the open, and is retired by rename then removal inside the owner's close and on both error returns of the owner's open.
- Opt-in marker lines (`KURU_OPEN_MARKERS=1`, stderr, `kuru-open-marker v1 <event> <monotonic_ns>`) replace the `Memory: ready.` signal for the open-time harness.
- User and developer documentation and every test that asserted a `Memory:` line are updated. No new token, argument or parse rule: unit 1's merged starter token is reused.
- Not in this change: a loading frame inside the terminal interface, and a sentence for the period after memory is ready while the provider and model list load (recorded in `tmp/roadmap/dx-followons.md`).

## Capabilities

### New Capabilities

### Modified Capabilities
- `versioned-memory`: observed open stage values (creation, upgrade, service start, contended-only ownership wait) and the owner's silent-to-caller open.
- `project-memory-owner`: owner shutdown order gains retirement of the activity record; new requirement for the tagged activity record.
- `chat-harness`: new requirements for the plain memory-open sentence and for the open marker lines.
- `public-documentation`: the memory startup documentation describes the one plain sentence instead of startup messages.

## Impact

- `packages/kuru-memory`: `src/progress.rs`, `src/service.rs`, new `src/service/activity.rs`, `src/facade.rs`, `src/store.rs` (three stage sites, `acquire_lock_reporting`), tests.
- `apps/kuru-tui`: new `src/memory_activity.rs`, `src/cli.rs`, `src/lib.rs`, tests `cli.rs`, `server.rs`, `unix_shell_turn.rs`, `terminal.rs`.
- Docs: `docs/memory.md`, `docs/configuration.md`, `docs/development.md`, `apps/kuru-docs/concepts/memory.md`, `apps/kuru-docs/reference/configuration.md`.
- BREAKING for scripts that search standard error for `Memory:` strings: they stop matching. No repository workflow or script does. The open-time harness moves to the marker lines.
- Because the owner retires immediately after the last client (unit 1), a back-to-back command can now show the waiting sentence while the previous owner closes.
- No new dependency, argument, protocol field or migration.

## Surfaces

- [x] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape

Interactive: the sentence and its terminal rendering. Deploy: the owner process writes and retires a new private file on every start, and the release binary gains an environment-variable marker output consumed by CI; Windows behaviour is unverified until native CI. Integration is not checked: no third-party contract changes. Agent behaviour is unchanged.
