## Why

Three Phase 1 surfaces stop one step short of the behaviour their capabilities
promise. `/cost` reports a bare `(incomplete)` subtotal without saying which
priced term it left out, so a reader cannot tell a missing token report from an
unmodelled cache-write rate. The terminal shows model, effort, mode, permission
counts and — only transiently, in the one-line status slot after a turn — a
context estimate, but it never shows a cost figure at all; it shows the hint
`N recorded calls · /cost` instead, so no single frame carries the session's
operating facts together. An unknown configuration key is rejected as a generic
`configuration type error`, even though the documentation promises a specific
forward-compatibility policy sentence, so the user is told that something is
wrong but not that a newer key needs a newer Kuru.

## What Changes

- The session estimate fold names each priced term it could not apply
  (long-context tier, cache-write rate, cached-input rate, invocation price,
  token components) and `/cost` renders those names beside the incomplete
  label. A tier that cannot be decided stays unapplied rather than half-applied;
  the stored raw per-kind components and frozen price basis are unchanged.
- The composer dock gains a standing meter row holding the last prepared
  request's context estimate with its window provenance and a labelled session
  cost estimate, so one frame carries model, effort, mode, cost, context use and
  permission state. An unknown price reads `cost unknown`, never a zero charge or
  a quota. Very narrow terminals abbreviate the row instead of dropping a
  control.
- An unknown configuration key is rejected with the documented
  forward-compatibility policy text rather than a generic type error, without
  echoing the offending key.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `configuration-schema`: the unknown-key rejection carries the documented
  forward-compatibility policy.
- `context-usage-accounting`: an incomplete estimate names the priced terms it
  did not apply.
- `chat-harness`: the standing composer controls carry cost and context use
  beside model, effort, mode and permission state.

## Impact

- `packages/kuru-core/src/accounting.rs` — new `UnappliedPriceTerm` enum and a
  defaulted `MoneyEstimate.unapplied` field on a read-fold type; no stored
  record shape changes and no migration.
- `packages/kuru-core/src/config.rs` — unknown-key rejections carry the
  documented policy; other configuration diagnostics are unchanged.
- `packages/kuru-memory/src/store/usage_ledger.rs` — `EstimateFold` records the
  reason with each incompleteness; the arithmetic is unchanged.
- `apps/kuru-tui/src/ui.rs`, `apps/kuru-tui/src/ui/render.rs` — `/cost` label
  and the new dock meter row.
- `docs/interface.md`, `docs/usage.md`,
  `apps/kuru-docs/guide/first-conversation.md` — user-visible descriptions.
- Tests: `packages/kuru-core/tests/config_schema.rs`,
  `packages/kuru-memory/src/store/usage_ledger.rs`,
  `apps/kuru-tui/src/ui.rs`, `apps/kuru-tui/tests/terminal.rs`.

## Surfaces

- [x] interactive — a user-visible/interactive surface (UI, TUI, CLI UX)
- [ ] deploy — deploy/runtime/CI-execution topology (infra, Dockerfile, workflow runtime, secrets, bind address)
- [ ] integration — a third-party/external contract (SDK, OAuth, schema/id-type reconciliation)
- [ ] agent-behavior — prompts, tools, model routing, or agent output shape
