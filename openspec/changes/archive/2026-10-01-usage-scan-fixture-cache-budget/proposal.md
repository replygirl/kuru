# Proposal

## Why

The `usage-scan-scaling` job (merged in #164) saves its aged fixture from
`main` to the repository's shared Actions cache without garbage collection:
about 340 MB at 1k and 2.1 GB at 5k conversations on ubuntu-latest, roughly
2.4 GB per entry, which crowds the Rust and bundle caches that every partition
restores. The fixture may occupy the shared cache only if, after `DOLT_GC` on
each store, both sizes together stay under about 1 GB; otherwise the job must
not cache it and ages in-job every run, which at 9 to 10 minutes stays under
the partition critical path of 14 to 17 minutes.

## What Changes

- Measure the post-`DOLT_GC` sizes of freshly aged 1k and 5k stores and apply
  the rule above, recording the numbers and the repository's Actions cache
  usage before the change.
- `.github/workflows/ci.yml` (`usage-scan-scaling`): either drop the fixture
  restore and save steps and age in-job on every run, or garbage-collect each
  store before a main-only save and refuse to save over the 1 GB budget,
  printing the sizes. The chosen branch is recorded in `tasks.md`.
- `packages/kuru-delivery/tests/release_workflow.rs`: the job's shape test
  follows the chosen branch.
- `packages/kuru-memory/src/test_support/usage_scan.rs` and its task comment
  in `packages/kuru-memory/mise.toml`: only if the fixture key stops being
  useful, the `key` subcommand and the key composition go with it.
- `docs/development.md` ("Usage scan scaling check"): the decision and its
  numbers.

## Impact

- Job `usage-scan-scaling` (required by `ci-gate`): its fixture handling and
  wall clock change; its measurement, bounds and assertions do not.
- Shared Actions cache: the fixture entry either disappears or is capped
  under about 1 GB.
- No secrets, permissions or other jobs change.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
