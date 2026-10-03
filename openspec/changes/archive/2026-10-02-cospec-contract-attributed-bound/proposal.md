# Proposal

## Why

`standalone_cospec_emits_one_document_and_preserves_every_gate`
(`packages/kuru-delivery/tests/cospec_contract.rs`) failed twice on
windows-latest coverage partition 5 (PR #158 run 36872976532 job 110405506019;
PR #176 run 37022142243 job 110887923249) with only
`standalone cospec timed out: Elapsed(())` at line 23 after 30 s. Its shared
helper wrapped every call in an outer `tokio::time::timeout`, so the panic
location never said which of the nine calls stalled, and on Windows that outer
bound dropped the future before `kuru_delivery::command`'s own timeout error
(phase, elapsed, pipe EOFs, owned-tree snapshot, cleanup, CPU/working-set
samples, stdout/stderr prefixes) could be built. The failure therefore cannot
be classified as a vendor stall (cospec/Bun, runner image) or ours (native pipe
or Job boundary); the fixture is defective as an instrument.

## What Changes

- Every cospec call goes through
  `kuru_delivery::command::output(&mut Command, 30 s)`: the same 30 s bound for
  each call (on Windows `output_with_timeout`, which carries the diagnostics),
  no retry, no sleep. Each call has a subcommand label; the `#[track_caller]`
  consumer panics with `standalone cospec <label> failed after <ms> ms:
  <full error>` at the calling line, or with the label, exit code and stderr
  when a call that must succeed exits unsuccessfully.
- Each finished call writes `cospec <label>: <ms> ms (exit …)` to the raw
  stderr handle, which libtest does not capture, so green coverage shards
  (run without `--nocapture`) build the per-call baseline in the job log. The
  `doctor --json` openspec-resolve evidence is written the same way.
- A sibling test drives the same helper against a new cross-platform
  `never-finish` mode of `kuru-delivery-fixture` with a 2 s bound and asserts
  the failure carries the label and `tool timed out`, plus on Windows the
  `read native stdout/stderr` phase, the arguments and the `samples=` field.
- `native-tests.yml` shard job: one `time cospec --version` step directly after
  the mise-action step that installs cospec, so the job's first execution of
  the executable is provisioning outside the test's bounds and its cost is
  logged. The fixture's private `XDG_CACHE_HOME` stays cold (no pre-warming).
- `release_workflow.rs` pins that step's position; `repo_validation.rs`'s
  incident fixture anchor follows the new step order.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

Test fixture and CI only: `packages/kuru-delivery/tests/cospec_contract.rs`,
`packages/kuru-delivery/tests/fixtures/delivery.rs`,
`packages/kuru-delivery/tests/release_workflow.rs`,
`packages/kuru-delivery/tests/repo_validation.rs`,
`.github/workflows/native-tests.yml`. No product code, bound or retry policy
changes. On a Windows timeout the failure now surfaces after the owned Job's
explicit termination and pipe joins (up to about 5-10 s after the bound)
instead of on drop; success paths are unaffected.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [x] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
