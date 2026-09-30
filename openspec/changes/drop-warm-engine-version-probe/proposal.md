# Proposal

## Why

Every ordinary project-memory open that finds the cached Dolt engine already installed (the
"warm" path, taken once each time a per-project memory owner process starts) spawns a `dolt
version` child process after fully hashing the cached executable, `LICENSES` and every notice
file. That process launch costs roughly 47-71 ms and, unlike the hash, buys almost nothing beyond
what the hash already proves: the cache directory is keyed by `DOLT_VERSION`
(`packages/kuru-memory/src/provision.rs`), so a mismatched Dolt version lands in a different
directory rather than overwriting a verified one, and the exact version of a freshly extracted
engine is already proven once, at install time, by the same probe before that cache is ever
activated. Paying the probe again on every subsequent warm open re-derives a fact the cache
layout and the install-time probe already established, on the one path (an owner process
starting) that runs frequently enough for the cost to be worth removing.

Attached opens (a client finding an owner already running) never pay this cost today and still
will not; nothing about first-launch extraction, which pays its own cold probe before activating
a freshly written cache, changes either. This is a small, deliberate narrowing of the warm-path
guarantee, not a general re-architecture of verification: it trades detecting "an executable that
hashes correctly today but no longer runs, or reports a different version, on this warm open" for
one fewer process launch on the hot path. That specific case is no longer caught until the SQL
server itself fails to start, at which point the open fails anyway — just later and with a
different error.

## What Changes

- The warm provisioning path (cached engine already installed) verifies the cache by hashing the
  executable, `LICENSES` and every notice file and by revalidating their private-object identities
  (`Directory::verify`), and returns without launching any process.
- `CheckedCache::probe` and its warm-path call site are removed; the `CheckingRuntimeVersion`
  progress stage is no longer reported on the warm path. Warm-path revalidation continues to run
  once, inside `CheckedCache::open_and_verify`, exactly as it does today; the second,
  probe-triggered revalidation is removed along with the probe it guarded.
- The cold path (extraction and first activation of a freshly written cache) is unchanged: it
  still probes `dolt version` before activating the newly extracted engine, and `CheckingRuntimeVersion`
  is still reported there.
- `openspec/specs/versioned-memory/spec.md` and `openspec/specs/embedded-runtime/spec.md` are
  amended so the durable text matches: the exact-version guarantee is stated as an install-time
  property of the cache, not a per-open one.
- User and contributor docs describing on-open verification are checked and updated in the same
  change so they do not describe a per-open version check that no longer runs.

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `versioned-memory`: "Managed full Dolt storage" no longer requires warm opens to re-probe the
  exact version; it still requires the full payload digest, identity revalidation, and that a
  corrupt cache fails explicitly. The "Offline cache" scenario is reworded to describe a
  digest-verified, version-pinned-by-install-time-probe cache rather than an "exact-version-checked"
  one on every open.
- `embedded-runtime`: "Safe local extraction" no longer requires the warm path to run a
  pathname-based version probe; existing cache verification is required to revalidate names and
  identities before returning the verified pathname instead. The "Concurrent warm cache
  verification" scenario is reworded so concurrent warm callers verify the complete pinned payload
  (not "and exact version") without waiting for the installation lock.

## Impact

- `packages/kuru-memory/src/provision.rs`: remove `CheckedCache::probe` and its warm-path call
  site (formerly `verified_cache_observed`, now the progress-free `verified_cache`, with
  `verify_existing_cache` losing its unused `progress` argument); cold-path probing
  (`provision_with_extractor_observed`, the cold `CheckingRuntimeVersion` report, and the
  cold-path probe struct/function) and the explicit `dolt_binary` probe are untouched.
- `packages/kuru-memory/src/provision/tests.rs`: the warm-stage assertion in
  `observed_provision_reports_actual_cold_warm_and_failure_stages` changes from
  `[VerifyingRuntimeCache, CheckingRuntimeVersion]` to `[VerifyingRuntimeCache]`; corruption tests
  keep asserting a rejected warm open (now via the hash alone).
- `packages/kuru-memory/src/provision/native_tests.rs`: stage assertions in the concurrent warm
  verification tests are checked and updated to match.
- `openspec/specs/versioned-memory/spec.md`, `openspec/specs/embedded-runtime/spec.md`: amended as
  above.
- `docs/*.md`, `apps/kuru-docs/**`: any passage describing on-open version verification is
  checked and updated to describe the install-time-only probe.
- No change to `startup_timeout_secs`, deadlines, retries, store creation, migrations,
  `service.rs`, progress relay beyond dropping the one warm-path stage emission, dependencies,
  toolchain or workflow files.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
