## Context

The parent change (`openspec/changes/archive/2026-09-18-shell-warmup`, this
branch's commit 68979a9) added `kuru_connectors::shell_warmup::warm_up_stock_powershell_engine`
and wired it into every `apps/kuru-tui` Windows test that reaches the
ToolHost stock shell, but explicitly left `packages/kuru-connectors/tests/windows_commands.rs`'s
two ToolHost-shell tests uncovered, naming the gap in its own proposal
rather than hiding it ("Adding warm-ups there is out of scope for this
minimal fix"). A follow-up review flagged that gap as blocking: those two
tests reach the same cold-start-prone `shell_inner` path through
`ToolHost::execute("shell", …)`, so main can still redden on Windows
through this file even with the parent change in place. This is not a new
root cause — it is the same PowerShell 5.1 engine/host cold-start
documented in the parent change and in `plan-flake-windows-installer.md` —
only a coverage gap in where the existing mitigation was applied.

## Goals / Non-Goals

**Goals:**
- Cover both uncovered `windows_commands.rs` tests with the existing
  warm-up, closing the named gap.

**Non-Goals:**
- Changing `shell_warmup.rs` itself, the shell tool's `timeout_ms`
  default/validated range, or any assertion in the two tests.
- Auditing or warming any other Windows test file — the review's finding
  named exactly these two tests and this file.

## Decisions

- **Call the warm-up directly, in-crate, rather than through a wrapper.**
  `windows_commands.rs` is an integration test of `kuru-connectors` itself
  (built with the `test-support` feature under `--all-features`, the same
  build that already provides `CARGO_BIN_EXE_kuru-connectors-windows-peer`
  to this file), so `kuru_connectors::shell_warmup::warm_up_stock_powershell_engine`
  is directly visible with no dev-dependency feature-unification hop —
  unlike `apps/kuru-tui`'s tests, which needed the `ensure_powershell_warm()`
  sync wrapper because their test functions are synchronous. Both
  `windows_commands.rs` tests are already `#[tokio::test] async fn`, so
  `warm_up_stock_powershell_engine().await` can be called directly with no
  wrapper at all — rejected adding one as unneeded indirection.
- **Call site: the first statement in each test body**, strictly before
  that test's first `host.execute("shell", …)`. Matches the parent change's
  placement rule (warm up ahead of the first timed shell call) exactly.

## Risks / Trade-offs

- [Risk] The warm-up call cannot be exercised on this macOS host (no
  `powershell.exe`, and cross-compiling `kuru-connectors` to
  `x86_64-pc-windows-msvc` fails on a pre-existing, unrelated host
  limitation — see `verification.md` §1.4) → Mitigation: the call is
  identical in shape to the already-landed, already-reviewed call sites in
  `apps/kuru-tui`'s tests; macOS compile/lint/type-check plus a structural
  diff review are the available local evidence, and the next Windows CI
  runs of `//packages/kuru-connectors:test` are the real signal, matching
  how the parent change itself was verified.

## Operational surface

No new bind address, container, secret, or binary version is introduced.
This changes CI-execution topology only in the narrow sense that two
`packages/kuru-connectors` Windows-shard test cases gain a bounded (130 s
own timeout, distinct from and not counted against the shell tool's
30 000 ms default `timeout_ms`) warm-up step before their existing timed
`tool shell` calls — the same runner, same job, same binary
(`x86_64-pc-windows-msvc`, matching every other Windows test in this
workspace) as before this change.
