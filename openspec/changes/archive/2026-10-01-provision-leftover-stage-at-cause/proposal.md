# Proposal

## Why

Main run 36930175874 failed in "Coverage partition (windows-latest, 5)", job
110597536882 (head 0e562595). The test
`provision::native_tests::concurrent_cold_windows_provision_publishes_one_verified_native_identity`
panicked at native_tests.rs:1670 because the version directory listed
`[".install-3UPCnG", ".leftovers", "x86_64-pc-windows-msvc"]`, not just the
target. Its final assertion asked for more than the product promises. Nothing
ever removes `versions/.leftovers` once a receipt has existed, so the exact
listing `[target]` fails after any retained stage, even one that a later sweep
collected. The documented contract (docs/memory.md, "Install stage teardown
ordering") only promises that a retained stage is receipted before the lock is
released and is never deleted on uncertainty.

The failure was also unclassifiable, and that is a product defect of ours. No
receipt, cause, OS error or attempt count reached the CI log. A sweep that was
refused again threw its cause away (provision.rs:1385). A warm open whose lock
attempt errored stayed silent (provision.rs:1439). `RemovalError` names only
the tree root, never the descendant that refused. Nothing recorded whether the
exited probe child's process object, which keeps the image of the executed
`private/probe/dolt.exe` mapped, was still alive when removal was refused. The
same family has now appeared three times in the flake catalogue (items 1 and 6,
and this run) without a measured mechanism. The holder is still unidentified:
on runner image win25-vs2026 20260925.250.1 Defender real-time monitoring is
disabled and C:\ and D:\ are excluded, so the catalogue's "AV/indexer" is
unsupported. A read of our own handles found none still open in the stage
(research §4).

## What Changes

- **The test asserts the contract.** The concurrent test now checks what the
  product promises:
  - every version-directory entry is the target, `.leftovers`, or a stage
    named by a receipt; an unreceipted stage is a hard failure;
  - each receipt is `published: true`;
  - its first cause belongs to an explicit table of causes that bounded
    recovery retries, and the attempt count is present (a cause recovery does
    not retry is a hard failure);
  - each receipt records at least one sweep refusal, and no sweep was
    skipped;
  - `.leftovers` remains only while a receipt remains, or with its kept-folder
    record;
  - the installation lock is released.

  Each receipt goes to the process's standard error, which the coverage shard
  inherits, so the evidence reaches the job log on a passing run too.
- **A sweep refusal is recorded.** A sweep whose single checked removal is
  refused rewrites that stage's receipt under the lock. It increments
  `sweep_refusals` and replaces `last_sweep_refusal` with
  `{phase, cause, os_error, descendant, probe_child_at_refusal, recorded_at}`.
  The original fields are kept and `version` stays `1`. A rewrite that fails
  keeps the old receipt and emits one `kuru.memory` record.
- **The refusing descendant is named.** `kuru_platform::fs::RemovalError`
  gains `descendant: Option<PathBuf>`, the failing entry relative to `path`.
  `path` keeps its documented meaning (the tree root). Retained-stage receipts
  carry it.
- **The probe child's state is measured at refusal (Windows).** The cold probe
  records its child's process id and creation time. At each refusal, the
  lease's exhausted window and every sweep refusal, Kuru opens that id with
  query-only rights and compares the creation time. The result is recorded as
  `retained`, `released` or `unknown: <cause>`. A new safe kuru-platform API
  provides this through existing imports, with no new `windows-sys` feature.
  Holder naming (handle enumeration) is out of scope (see design).
- **An empty `.leftovers` is removed.** A sweep that leaves no receipt, where
  only an empty `staging` remains, attempts one checked removal of
  `.leftovers` under the lock. A refusal leaves it for the next sweep and
  emits one `kuru.memory` record. A `staging` that still holds a record is
  never removed.
- **A failed warm lock attempt is reported.** `try_cache_lock` returning
  `Err` on a warm open emits one `kuru.memory` record. A busy lock (`Ok(None)`)
  stays silent, and warm opens still never wait.
- **Sweep tests run on every OS.** The direct-sweep tests move from the
  Unix-only module into one that compiles on every OS, with refusal fixtures
  for both. Windows gains a sweep refusal test, and the cancelled-activation
  test collects its stage through the product sweep instead of
  `remove_dir_all`.

Unchanged: `LOCK_TIMEOUT`, `CLEANUP_RETRY_LIMIT`/`SPACING`,
`ACTIVATION_RETRY_LIMIT`, one attempt per receipt per sweep, the receipt and
diagnostic before lock release, no deletion on uncertainty, warm opens never
waiting, `LEFTOVER_STAGE_CAP` reporting only, private-object validation and the
owner-private requirement of `remove_tree`, the field drop order of
`StagedActivation`, and the boundary of unsafe code.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `embedded-runtime`: MODIFIED "Install stage teardown ordering" (a receipt
  names the refusing descendant and, on Windows, the probe child's state at
  refusal; recording never changes the resolution). ADDED "Leftover stage
  collection" (each lock acquisition attempts every receipt once; busy lock;
  refusal recorded; empty `.leftovers` attempted once; failed lock attempt
  reported).

## Impact

- `packages/kuru-platform/src/fs.rs`, `fs/unix.rs`, `fs/windows.rs`:
  `RemovalError::descendant`.
- `packages/kuru-platform/src/windows/process.rs`: `ProcessStamp`,
  `NativeChild::stamp`, `process_object_retained` (audited Windows interop,
  existing imports).
- `packages/kuru-memory/src/engine.rs`, `provision.rs`: probe-child stamp,
  receipt fields, sweep refusal recording, `.leftovers` lifecycle, the two
  new `kuru.memory` records (admitted fields `stage`, `first_cause`,
  `os_error` only, so `apps/kuru-tui/src/diagnostics.rs` is unchanged).
- `packages/kuru-memory/src/provision/native_tests.rs`, `tests.rs`, new
  `sweep_tests.rs`.
- `docs/memory.md`: collection wording and the receipt's refusal record.
- No dependency, lockfile or `windows-sys` feature change.

## Surfaces

- [ ] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
