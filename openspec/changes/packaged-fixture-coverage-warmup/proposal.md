# Proposal

## Why

`packaged_install_and_update_preserve_complete_offline_memory`
(`apps/kuru-tui/tests/embedded_runtime.rs`) is on `main` at `b9453c5b` (#166)
red on windows-latest coverage partition 2: its first timed launch still
extracts and probes the embedded engine and builds the per-machine store
template under one 30 s client wait, and under coverage instrumentation that
cold path's measured 10-12 s uninstrumented Windows cost (install job logs)
sits close enough to the deadline to miss it ("memory service readiness
deadline exceeded"). #168 already fixed the analogous flake in the native
`windows_mise` fixture by warming its engine cache and store template before
the timed launch; this change applies the same warm-up, but only under
coverage, to the packaged `embedded_runtime` fixture — the test process
provisions the fixture's own cache and builds its own store template with the
installed packaged binary as supervisor before that binary's first launch is
timed, and verifies the launch actually used the warm template. Per the
lead's ruling (quoted in the PR body), this is correct test design, not a
workaround: an instrumented build is not a product condition, and the actual
product-side defect — a flat wait that abandons progressing work — is a
separate, already-landing readiness-window change.

## What Changes

- `apps/kuru-tui/tests/embedded_runtime.rs`: a pure `launch_mode` function
  (parameterized on an `Option<&OsStr>`, never reading the process
  environment itself) decides `Warm` only when `LLVM_PROFILE_FILE` is set;
  `Installation::conversation` keeps its unchanged cold `ensure!` first, then,
  only in `Warm` mode, provisions the fixture's own cache
  (`kuru_memory::provision::provision`) and warms its store template
  (`kuru_memory::test_support::warm_template_cache`) with the installed
  packaged binary as supervisor, under `.context(...)`, before starting the
  timed clock; after the launch it asserts `TemplateCacheReceipt::verify_used`
  and that the store's recorded template key equals
  `kuru_memory::test_support::template_key()`. The printed first-launch line
  states `cold` or `warm (coverage)` from the same mode value that gated the
  warm-up. `Cold` mode (the install job, every OS, uninstrumented) is
  byte-for-byte unchanged in behavior and assertions.
- A unit test for `launch_mode` pinning `Some(_) -> Warm` and `None -> Cold`,
  deterministic and independent of the ambient test-runner environment.
- `docs/development.md`: one sentence beside #168's native-mise-fixture
  paragraph stating the packaged fixture warms its cache under coverage only,
  the same way, and stays cold (unconditionally) in the install job on every
  OS.

## Impact

Test-only. Touches `apps/kuru-tui/tests/embedded_runtime.rs` and
`docs/development.md`. No product code, no CLI/config/workflow surface, and
no change to `startup_timeout_secs`, any deadline, retry behavior, or the
native `windows_mise` fixture #168 already warmed. Inference, unmeasured
until this change's own PR CI: this should turn the windows-latest coverage
partitions green for this test without a rerun, since #168's analogous
warm-up on `windows_mise` did so for that fixture. The Installation job on
every OS remains the cold proof that an installed artifact unpacks its
engine and builds its store template in one launch.
