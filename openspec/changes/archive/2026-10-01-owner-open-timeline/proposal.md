# Proposal

## Why

A reopen of an existing project on a macOS arm64 release build spends about 249 ms between the engine supervisor reporting Ready and the service endpoint being published (`tmp/roadmap/unit6-owner-start-split-macos-2026-10-01.md`). That stage is a sequence of pool opens, version and validation reads, candidate recovery and two usage-ledger scans, and nothing observable says which of them dominates or which grow as a store ages. Optimizing without that split would be guesswork, and the open-time harness can only see the stage as one interval.

This change adds a measurement-only instrument: an owner-side, environment-gated open timeline, and a package-owned way to age an isolated project store through the real write paths so the timeline can be read on fresh and aged stores. Nothing is optimized here; any row that grows with age becomes its own later unit, designed with the lead first.

## What Changes

- With `KURU_OPEN_TIMELINE=1` (exactly `1`) in the memory service owner process, the owner stamps about eighteen named open milestones as nanosecond offsets from one monotonic anchor, in a bounded in-memory log sealed at endpoint publication, and writes the log once after it has released its owner lock in close, as a non-durable owner-private file `open-timeline-<service-generation>.json` beside its endpoint. With the variable unset there is one environment read and an untaken branch per site, and nothing else changes.
- No supervisor frame, client marker or progress stage changes. The supervisor is spawned with a cleared environment and stays inert. Windows owners do not inherit the variable and are out of scope.
- `kuru-memory` test-support gains an aged-store fixture, reached by an `age-store` arm of the package binary under the `test-support` feature and an opt-in `measure:age-store` mise task. It ages one existing isolated store with thousands of conversations and usage rows through the facade session lifecycle and the usage ledger, deterministic for a seed and size. It is never in the shipped binary.
- The macOS measurement, taken outside the repository against a release binary, is recorded in `tmp/roadmap/unit6b-notes-2026-10-01.md`. Nothing from the optimization candidates R1-R3 is implemented.
- `docs/development.md` documents the variable, the file and the fixture task.

## Capabilities

### New Capabilities

### Modified Capabilities
- `project-memory-owner`: new requirement for the owner open timeline, whose write follows the existing close order after the owner lock is released.

## Impact

- `packages/kuru-memory`: new `src/open_timeline.rs`; one-line stamps in `src/service.rs`, `src/server.rs`, `src/store.rs`, `src/store/usage_ledger.rs` and `src/provision.rs`; `validate_branch` in the usage ledger returns its decoded row count; a test-only owner-environment helper in `src/service/activity.rs` becomes crate-visible; new `src/test_support/aged_store.rs`, `src/test_support.rs`, `src/main.rs` (feature-gated arm) and `mise.toml`.
- Docs: `docs/development.md` only. The variable is not user documentation.
- Not breaking. No new dependency (`serde_json` and `uuid` are already used), argument, protocol field, migration or workflow.
- A gated owner leaves one 1-2 KiB file per run in its owner-private services directory; nothing reads or removes them.
- Deliberately unchanged: the 90% line gate (the module and sites are covered by the tests in `verification.md`), `startup_timeout_secs`, retry and deadline behaviour.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
