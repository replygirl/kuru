# Design

## Context

Measured evidence comes from run 36930175874, job 110597536882, windows-latest
partition 5, image win25-vs2026 20260925.250.1. It records only the panic
listing at native_tests.rs:1670. No retained-stage diagnostic, receipt, cause or
`os error` line appears in the job log or in the uploaded diagnostics artifact.
The test registered no retained-stage observer, and the receipt stayed in the
runner's temporary directory. The research and design notes record this in
`tmp/roadmap/provision-leftovers-{research,design}-2026-10-02.md`, which are not
part of the repository.

The following is inferred from code with high confidence:

- The retained `.install-*` directory is the winning installer's own
  published stage. Only a lock holder that finds no destination creates a
  stage (provision.rs:135-148).
- Its bounded window (`CLEANUP_RETRY_LIMIT`, 2 s) exhausted. The loser's cold
  sweep and the warm open's sweep each refused once more.
- After publication the stage holds only `private/probe/`: the executed
  `dolt.exe` copy and the probe's home directories.

A read of every handle our path opens inside the stage found none still open
at `close_or_keep`. Two candidates remain for the holder, neither confirmed:

- a system-side reference to the executed image;
- another process holding the exited probe child's process object. That object
  keeps its image section referenced, which yields native error 5 on the
  disposition delete, the class that files.rs:700-705 already names.

Two defects of ours are certain:

1. **The test was wrong.** `.leftovers` is never removed, so the exact listing
   fails after any receipt.
2. **The evidence was missing.** A refused sweep discarded its cause, a failed
   warm lock attempt was silent, `RemovalError` cannot name the refusing
   descendant, and nothing measured the probe child's process object.

## Goals / Non-Goals

**Goals:**

- The concurrent test fails only on a real contract breach. It surfaces every
  receipt in the CI log on both passing and failing runs.
- Every refusal records what classifies its next occurrence: the phase, the
  native error, the descendant, the attempts and elapsed time, and, on Windows,
  whether the probe child's process object was still alive.
- An empty `.leftovers` no longer outlives its last receipt.
- The sweep has Windows tests and tests that run on every OS.

**Non-Goals:**

- **Naming the holding process.** This means the design's `fs::holders`
  handle-enumeration walk and the own-handle guard. The critique found that
  `FileProcessIdsUsingFileInformation` and the Restart Manager both enumerate
  open file handles or the modules of running processes. Neither can see an
  image section that an exited process object still references, which is the
  mechanism the error class points to. The walk would therefore add an audited
  unsafe surface and a `windows-sys` feature while passing vacuously for this
  family.

  The lead ruling's alternative is applied instead: the receipt records the
  refusal's OS error, the attempt count and the timing, plus the probe-child
  measurement, which needs no new feature. The gate for a later attribution
  change is a receipt with `os_error: 32` or a `retained` probe child.
- **Retrying, lengthening a bound, or making a warm open wait for the lock.**
- **Changing how a stage is resolved.**

## Decisions

- **The probe child's process object is the instrument.**
  - The cold probe records the child's `ProcessStamp { id, created }` from
    `GetProcessTimes` on the retained handle.
  - At each refusal, `process_object_retained` opens that id with
    `PROCESS_QUERY_LIMITED_INFORMATION` and compares the creation time.
  - `ERROR_INVALID_PARAMETER` from the open means no process object has that
    id, so the result is `released`. A different creation time means the id was
    reused, also `released`. An equal creation time is `retained`. Anything
    else is `unknown: <cause>`.
  - Our own handles to the child are closed before release:
    `verify_version_with_timeout` drops its `Child` before the probe thread
    sends (provision.rs:556-573). So `retained` means another process holds
    the object.
  - That an exited process's object stays openable by its id while any handle
    to it is open is documented Windows behaviour. It is inferred here, not
    measured on this host.
  - Alternative rejected: handle enumeration (see Non-Goals).
- **The descendant is carried as a cursor.**
  - Both native `remove_tree` bodies push each entry's name before acting on
    it and pop it on success. An error therefore leaves the cursor at the
    failing entry, or at the directory being enumerated.
  - `RemovalError::descendant` is that relative path, `None` for the root.
  - `Display` is unchanged, so no cause text that a test or an operator matches
    moves.
  - Alternative rejected: changing `path` to the descendant. That would break
    its documented root meaning, which callers rely on for reconciliation.
- **Refusals are recorded by rewriting the receipt in place.**
  - The already-read bytes are parsed as a JSON map, so unknown fields survive.
  - `sweep_refusals` is incremented, and `last_sweep_refusal` is replaced
    rather than appended to, so the size stays bounded.
  - The result is checked against `LEFTOVER_RECEIPT_LIMIT` and written through
    `files::write` (`Publication::ReplaceRegular`). The write is under the
    lock, the only writer.
  - Alternative rejected: a diagnostics record per refusal. Every warm open
    after a refusal would emit it, and the evidence would not reach the
    receipt.
- **`.leftovers` removal is narrowly conditioned.**
  - It happens only when no `.json` remains and the folder holds nothing but an
    empty `staging` directory.
  - It is one checked `remove_tree` on the owner-private handle the sweep
    already opened.
  - `files::write` keeps a temporary record in `staging` on an uncertain
    publication as evidence, so a non-empty `staging` is never removed.
- **New `kuru.memory` records use admitted fields only.** These are `stage`
  (the `.leftovers` path), `first_cause` and `os_error`, so the TUI
  diagnostics allowlist is unchanged.
- **The test's cause table is keyed by `attempts`.**
  - Every cause that bounded recovery does not retry returns before
    `wait_for_cleanup_retry` (files.rs:533, :568, :627 and the identity
    `ensure!`s). So `attempts` is present exactly when the first cause was a
    retried one.
  - The test then matches that first cause against the explicit table of
    retried classes:
    - `Rejected` removal with 5 or 32;
    - any `Uncertain` removal, including the `null`-errno occupancy check;
    - the pending-child, remained-child and container-remained seeds;
    - an outer pending open with 5 or 32;
    - an outer `remove_dir` with 32 or 145.
  - The test prints the matched row. A missing `attempts` or an unmatched cause
    fails.
- **Evidence goes to stderr.** Libtest capture hooks only the
  `print!`/`eprint!` machinery, and the coverage shard inherits the test
  binary's stderr (packages/kuru-delivery/src/coverage.rs:1427, :1454). The
  test therefore writes receipts with `std::io::stderr().write_all`. No
  `--nocapture` is added.

## Risks / Trade-offs

- **[Unattributed residual]** If the probe child is `released` and the refusal
  is a retried class, this change cannot classify further, and the test
  passes with the receipt printed. → Recorded in the flake catalogue as a
  count, not hidden. The next evidence step is its own change.
- **[A retained probe child that is ours]** If a later change leaked our own
  process handle to the child, `retained` would also appear. → Our handle's
  lifetime is structural (`Child` is dropped inside `verify_version`). A
  `retained` reading would open that question, and this design names it as
  the gate for the attribution change.
- **[Concurrent test fails without a defect]** A transient hold on `.leftovers`
  itself would leave it present without a receipt. → The kept-folder record
  makes that case pass with its cause printed, and no other case exists
  without a product breach.
- **[Receipt rewrite failure]** → The old receipt stays valid, the stage
  counts as remaining, and one diagnostics record names the cause.
- **[Older Kuru reading a new receipt]** `StoredLeftoverStage` reads only
  `version` and `stage`, so the extra fields are ignored. `version` stays
  `1`.
