# Design

## Context

`classify_historical_attempts_in` (`packages/kuru-memory/src/store/migrations.rs:1502-1573`)
walks every reserved migration branch at or below the store's current/found
version and, for each one, opens a pool on the branch and (clean path) a
second pool on its sole parent commit via `validate_commit_version`
(`migrations.rs:1601-1610`) to run full schema validation there. `ancestor`,
`sole_parent` and the two `dolt_log` lookups already run on the caller's main
pool. The per-connection project/instance identity check in `server.rs`
refuses a pool whose row differs from the opener's, so any retained branch
with a different instance identity (a template-born branch) cannot be
classified through a branch/parent pool at all — only through main.

The engine-contract spike (`tmp/roadmap/store-creation-design/engine-contract-findings.md`,
item S8) measured, on macOS with Dolt 2.3.5, that every inline check has a
main-pool equivalent except full schema validation at the head and at the
parent: `information_schema.columns` scoped from main for a non-`DATABASE()`
schema (`kuru/<branch>`) returns zero columns, while `SHOW CREATE TABLE ...
AS OF '<hash>'` against main returns identical table definitions to the
branch pool. The spike did not try a detached `USE kuru/<hash>` connection
from the main pool followed by the validator's existing
`DATABASE()`-scoped `information_schema` queries — that is this change's
first implementation task, tried in the order the brief gives, because if it
reproduces `validate_version_with` exactly it lets the classifier reuse the
validator's existing queries unchanged rather than hand-rewriting each one
against `SHOW CREATE TABLE` or a revision-qualified `information_schema`.

## Goals / Non-Goals

**Goals:**
- State the fail-closed rule precisely enough that the implementer cannot
  read a passing "happy path" test as license to loosen it.
- Record the decision tree for the parent-schema form so the first
  implementation task has an unambiguous stopping rule.
- Record the parity-or-stop gate as a design commitment, not just a test
  name, since it changes delivery order if it fails (proposal's Capabilities
  section: no spec delta unless parity fails).

**Non-Goals:**
- Re-deciding *what* is validated, or removing any check
  `classify_historical_attempts_in` makes today (proposal's Impact section).
- The template/machine-cache work itself, or `discover_current_attempt_in`
  (the current-attempt discovery path, not historical classification) —
  out of scope for this change.
- Choosing the parent-schema SQL form here: that is measured, not designed,
  and belongs in the findings file the first task writes.

## Decisions

**D1. Reuse the classify-caller's existing main pool; do not add a second
"classification pool" concept.**
`validate_active`, `validate_inspection` and `validate_ready` already hold a
`&MySqlPool` for main. `upgrade_in`'s per-step loop holds the same. The new
classifier takes that pool as a parameter, exactly like `ancestor` and
`sole_parent` do today. Rejected alternative: a dedicated read pool opened
once per classification pass — this would still count as a pool per the
proposal's pool-count metric and adds a lifecycle (open/close) the current
design has no reason for.

**D2. Settle the parent-schema form by the exact order the brief specifies,
stopping at the first exact match.**
Try, in order: (1) `USE kuru/<hash>` on a detached main-pool connection with
the unmodified `validate_version_with`/`validate_schema_with` queries; (2)
`SHOW CREATE TABLE ... AS OF`; (3) `information_schema` with `AS OF` or a
revision-qualified database name. "Exact match" means: same verdict as
`validate_version_with` on the branch pool, for a valid parent and for a
parent with a wrong schema (a negative case, not just the happy path), for
both `root` and `kuru_reader`. Record the chosen form and why the earlier
options were rejected (or why the chosen one was the first tried) in
`tmp/roadmap/store-creation-design/` findings, since that file is the
project's record of measured engine behavior and later changes (the
template work) read it. Rejected outright: comparing only table names
instead of full definitions — the proposal and AGENTS.md both rule this out
as a weakened check.

**D3. Fail closed on any unexpected row shape, uniformly across every new
query.**
A main-pool query that returns no row, more than one row where one is
expected, or a null/mismatched column is treated exactly as today's code
treats an inconsistent branch: `ensure!` fails the check, the branch is
reported as not classifiable/ambiguous, never as published. This is not
branch-state-specific — it is the same discipline `sole_parent` already
applies (`parents.len() == 1`) extended to every new main-pool read.
Rejected alternative: treating a missing row as "branch absent, skip" for
some checks — this would silently change which branches get classified,
which the proposal's Impact section rules out.

**D4. Keep the pool-based classifier under `cfg(test)` only if the parity
test needs it as a live oracle; otherwise remove it.**
`main_pool_classification_agrees_with_branch_pool_classification` (tasks.md)
needs a second, independently-written opinion to diff against. If keeping
the current `classify_historical_attempts_in` body (renamed, not exported)
under `#[cfg(test)]` is the simplest way to get that second opinion, do so
and say so in the PR description and tasks.md evidence line — AGENTS.md's
coverage rule permits this explicitly ("remove the old pool-based
classification from product code unless the parity test needs it, in which
case keep it under `cfg(test)` and say so"). Do not keep any pool-based path
reachable from product code paths (`validate_active` etc.) after this
change lands.

## Risks / Trade-offs

- [Risk] The first task finds no exact-match form for parent-schema
  validation (all three fail the negative case, or diverge for the reader
  role). → Mitigation: this is exactly the stop condition the proposal and
  tasks.md name; the task records the finding and the change reports
  `ok=false` with the reason rather than shipping a weakened check. This is
  a known possible outcome, not a design flaw to paper over.
- [Risk] Parity cannot be shown for some branch state (dirtied working set,
  force-moved ref, wrong receipt, wrong parent schema, differing instance
  identity) because the main-pool read for that state behaves differently
  than the S8 spike's fixtures suggest. → Mitigation: tasks.md schedules the
  parity test per state before the call-site swap, so a divergence is found
  before any product code changes, and the task list has an explicit stop
  instruction for this case.
- [Risk] Removing the pool-based classifier from product code but keeping a
  copy under `cfg(test)` lets the two implementations drift silently over
  time (the test oracle stops reflecting what product code would have done).
  → Mitigation: the parity test is not one-shot — it stays in the suite
  (tasks.md), so any future change to the main-pool classifier that breaks
  parity with the frozen oracle is caught by CI, not just by this change's
  author.
