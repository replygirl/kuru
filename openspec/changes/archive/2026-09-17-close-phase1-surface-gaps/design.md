## Context

The three defects share a root cause: each surface stopped at the point where a
value existed internally but was never carried to the place the user reads.

- The estimate fold already knew why a subtotal was incomplete — it sets
  `incomplete = true` at nine distinct branches — but `MoneyEstimate` had no
  field to carry the reason, so `/cost` could only print `(incomplete)`.
- `SessionUsage` and the per-request `ContextEstimate` were both already on the
  `View`, and the dock already rendered permission counts, but the dock rendered
  neither of those two values; the context estimate reached only the one-line
  transient status slot and the cost reached only a `/cost` transcript message.
- The unknown-key rejection dropped the underlying `toml::de::Error` to keep the
  offending key out of the message, and with it the fact that the rejection was
  a forward-compatibility one rather than a type error.

## Goals / Non-Goals

**Goals:**

- Name the priced terms an incomplete estimate did not apply.
- Put cost and context use in the same persistent frame as model, effort, mode
  and permission state, with values that match `/cost` and `/permissions`.
- Make the unknown-key rejection say the documented policy.

**Non-Goals:**

- No change to permission semantics, accounting arithmetic, stored record shape,
  provider code, or any configuration key.
- No new dependency, and no schema migration: `MoneyEstimate` is a read-fold
  result, never persisted.
- No replacement of the transient status line; it keeps carrying active work.

## Decisions

- **A typed term set, not a free string.** `UnappliedPriceTerm` is a small
  `Copy` enum with a `label()`; the fold collects it in a `BTreeSet` so the
  rendered order is stable and duplicates collapse. A `Vec<String>` was rejected
  because it would let the fold invent wording per call site.
- **History gaps are not priced terms.** `!historical_complete` and
  `record.incomplete` keep setting a bare `incomplete`, because `/cost` already
  discloses both on their own lines. Naming them here would have made the new
  list mean two different things.
- **The meters go in the dock, not the status line.** The status line is the
  transient slot, by design; only the dock is present in every frame. Adding one
  dock row costs one terminal line and keeps the permission row last, so the
  height-clamped truncation order is unchanged.
- **Narrow widths abbreviate rather than drop.** Below a 40-column dock the row
  collapses exact counts to thousands and replaces the provenance word with a
  `~` marker, keeping all six facts present at the 40-column PTY width. Dropping
  a token was rejected: the acceptance bar is co-presence.
- **Unknown price reads as a word.** `cost unknown` rather than `$0` or a blank,
  and a known-but-incomplete subtotal reads `≥$… est` so the lower bound is
  explicit. `/cost` remains the full, unabbreviated statement.
- **The policy sentence is matched, not re-derived.** The rejection detects the
  `unknown field` case from the serde error and substitutes the documented
  sentence; the key itself is still never echoed, and an out-of-range value
  keeps its own message.

## Operational surface

Terminal-only. No bind address, container, runner, secret, connection limit or
binary/arch selection changes. The new dock row adds one line to the composer at
every width and uses glyphs already shipped in the dock (`◆`, `≈`, `·`, `~`);
it reads no credential store and performs no I/O of its own, sourcing both
values from state the view already holds. Very narrow terminals abbreviate the
row, so the minimum supported terminal size is unchanged.

## Risks / Trade-offs

- [The extra dock row shortens the conversation area by one line] → The row is
  counted in `draw`'s control-row budget at every width branch, and the existing
  40×18 real-PTY case is asserted to still render every control.
- [The dock's cost value could drift from `/cost`] → Both come from one
  `Harness::session_usage()` refresh at the end of every dispatch, including the
  `/cost` dispatch itself, and the new PTY test asserts agreement in one frame.
- [A future incompleteness branch could forget to name its term] → Every branch
  now goes through one `EstimateFold::mark` helper that sets the flag and the
  term together, so the flag cannot be set without a reason.
