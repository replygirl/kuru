# Proposal

## Why

`MemoryStore::temporary()` failed on main run 37174195183 (449dca9e, #206, which
changed only kuru-runtime test files), ubuntu-latest coverage partition 7, job
111354197802: `dolt_tests::reconciliation_publishes_only_durable_choices_before_the_next_mutation`
panicked at `dolt_tests.rs:953` with "open active memory server", "memory
supervisor exited unsuccessfully (exit status: 1)" and "Dolt database bootstrap
failed while checking the bootstrap data directory: Dolt bootstrap data directory
mismatch". The fixture's `server.log` held only "Kuru engine shutdown: Graceful";
the partition ended 16 passed, 1 failed in 4.51 s. It is the first occurrence and
main is red on it.

The check that fired is `initialize_database` in
`packages/kuru-memory/src/server.rs`: it reads `@@datadir` from the server that
accepted the supervisor's authenticated connection and compares its directory
identity with `<fixture directory>/data`. The supervisor reserves a loopback port
by binding port 0 and dropping the listener, then starts Dolt on that port. Its
readiness loop (`start_database`) checks `try_wait`, then connects to the port
with the store password. Copies of the pre-migrated test template share that
password (docs/development.md), so a different fixture's Dolt that has taken the
reserved port in the window after the listener was dropped authenticates the
probe. The probe then reads that server's `@@datadir`, which cannot equal this
fixture's, and the supervisor fails permanently. The designed recovery for a
taken port (`OWNED_START_ATTEMPTS`, keyed on Dolt's own "Port N already in use."
and a `DoltPrematureExit`) never runs, because the mismatch error returns before
the supervisor's own Dolt has had the chance to exit.

Labelled inference: the log does not show which server answered. The cause rests
on the code path (the only way `@@datadir` of an authenticated server differs
from the fixture directory is a different server answering on the port), on the
shared template credential, and on the 16 concurrent Dolt-using tests in the
partition. The empty Dolt output in `server.log` fits a child stopped before it
logged its bind failure. The planned regression test reproduces the path
deterministically and so confirms or refutes the inference.

## What Changes

- `start_database` treats a data-directory mismatch as evidence of a foreign
  listener, not as a bootstrap failure: it closes the pool, writes nothing to the
  foreign server, remembers the mismatch, and keeps the existing loop, which
  observes the supervisor's own Dolt exit (`DoltPrematureExit`) so the existing
  bounded, finite selected-port retry in `supervise_with_port_hook` reselects a
  port. If the supervisor's own Dolt stays alive and the answering server stays
  foreign, the deadline fails with the mismatch in its context.
- The check itself is unchanged and still precedes every write: nothing is ever
  bootstrapped, adopted or granted on a server whose data directory is not this
  store's. No retry is added around the whole open, and no validation is relaxed.
- Regression test in `kuru-memory` `server_tests.rs`: a foreign authenticated
  Dolt (same password, different directory) is made to hold the selected port
  through the existing port hook; the supervisor must reselect a port, become
  ready on its own data directory, and leave the foreign server untouched. It
  fails against the current code with the mismatch error.
- Catalogue update in the untracked flaky-test notes with the cause.

## Capabilities

### New Capabilities

### Modified Capabilities

## Impact

- `packages/kuru-memory/src/server.rs` (`start_database`, and a typed mismatch error from `initialize_database`)
- `packages/kuru-memory/src/server_tests.rs`
- A sentence in `docs/development.md` or `docs/memory.md` only if the supervisor's documented port-collision behaviour names the retry trigger.
- No dependency, schema, store-format or other crate change; `kuru-runtime` is untouched.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
