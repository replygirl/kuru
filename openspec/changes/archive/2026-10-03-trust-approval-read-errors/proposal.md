# Proposal

## Why

When the operating system refuses or fails a read of a workspace approval
record (for example `EACCES` after a permission change, or `EIO`), Kuru
mislabels the failure. `ApprovalStore::inspect` and `inspect_nested` fold every
read error into `ApprovalState::Invalid`, so `trust status`, preflight and the
nested review show "approval state is invalid or unsafe" and point at
`kuru trust approve`, which repeats the same failing read. The nested
instruction gate's `publish()` turns the same read error into
`unwrap_or(false)`, reporting "workspace approval changed during review" when
nothing changed. Both paths already fail closed; the defect is a misleading
diagnosis and a remedy that cannot work (fixed-wait audit item 9,
`tmp/roadmap/fixed-wait-audit-2026-10-02.md` section 3B rank 4, section 5 U4b).

## What Changes

- `ApprovalState` gains `Unreadable(ReadFailure)`. `ReadFailure` holds only a
  raw OS error code and displays the platform's standard message for it; it
  never records bytes or path components. `inspect` and `inspect_nested`
  return it for a genuine OS read failure; any other failure, including the
  checked filesystem's synthesized structural rejections and OS refusals of
  object shape (a symlink under no-follow, a directory or file in the other's
  place, a missing object), stays `Invalid`.
- `Unreadable` is non-approving exactly where `Invalid` is: it is never
  `Matching`, and `inspect_nested` returns no review generation for it, so a
  persistent nested choice is still refused.
- The CLI status table names the I/O error and the remedy (restore the
  permissions of the approval record or the Kuru data directory, then retry)
  instead of suggesting approval.
- The nested instruction gate's `publish()` propagates the read error with the
  context "workspace approval record could not be read"; "changed during
  review" remains for a successfully read, different generation.
- No change to what the gate approves.

## Capabilities

### New Capabilities

### Modified Capabilities

None. `workspace-trust` requires invalid, unsafe or uncertain approval state to
fail closed before activation, and it still does; the requirement does not
specify the diagnostic text that this change corrects.

## Impact

- `apps/kuru-tui/src/trust.rs`: `ApprovalState::Unreadable`, `ReadFailure`,
  `failed_read_state`, the `Err` arms of `inspect` and `inspect_nested`, tests.
- `apps/kuru-tui/src/instruction_gate.rs`: `ProposedActivation::publish`
  generation check, test.
- `apps/kuru-tui/src/cli.rs`: the `manifest_text` status table row.
- `apps/kuru-tui/tests/cli.rs`: CLI regression test.
- `docs/configuration.md#workspace-trust`: the unreadable-record behaviour.
- No new dependency (`nix` is already a Unix dependency of `kuru`), protocol,
  record format or migration.

## Surfaces

- [x] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
