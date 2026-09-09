## Context

The first pass added a colorful welcome and a cached conversation renderer, but
uses the same circular peer graph for every framework and places settings far
from their keys. View advances only for work or a finite welcome sequence.
Mode is saved in session state; new startup config does not consult interactive
preferences. Model and effort changes currently mutate only live config.

## Goals / Non-Goals

**Goals:** Extend the visual promise of the welcome into a coherent daily workflow;
verify the whole selection/restart loop; isolate ornamental motion from runtime facts.

**Non-Goals:** Claiming physiology or consciousness, new providers/protocols,
credential handling, full Markdown compliance, automatic chat resumption or a
dependency/theme-system expansion.

## Decisions

- Use an open ink canvas and a bounded welcome composition. The existing wordmark
  has stable letter colors; decorative ASCII contours provide depth. Repeating
  rounded panels and a full-width rainbow divider were rejected because they compete
  with the stronger identity visible in the user's screenshot.
- Put model, effort and framework values beside F2/F3/F4 in one composer dock.
  A separate factual status line conveys feedback, counts and operation state;
  redundant Message/Conversation/ready labels and exposed motion controls disappear.
- Use an orbital IFS field, layered polyvagal traces, a Freudian triangle and a
  Jungian rosette. Scenes share typography, color semantics and dot/contour language,
  with live route edges drawn separately from decorative contours. An identical
  circle with new labels would not establish distinct framework identities.
- Provide low-rate ambient frames and a brief editing pulse at a faster bounded
  cadence. Use supplied elapsed time and stable geometry, not randomness or a
  repaint loop driven by wall-clock reads inside drawing. Preserve cached transcript
  layout; terminal focus loss can suspend ornamental work.
- Store project preferences as one typed SQLite value, separate from mode/session
  namespaces. Reusing session state would require --resume and fail fresh-launch
  expectations. Changing project config files would introduce unrelated file edits.
- Merge remembered choices after user/ancestor defaults but before explicit local
  config and CLI flags. Provider model/effort selections form a pair; changing a
  model explicitly must not carry an incompatible remembered effort. Reading a
  session or using an invocation override never rewrites remembered choices.
- Verify real PTY restart behavior, complete control states, four rendered scenes,
  input/paste animation and frame cost. Numeric coverage complements these observed
  flows; it does not replace visual inspection.

## Risks / Trade-offs

- [Continuous motion consumes CPU] → Low idle cadence, bounded scene size, cached
  text, focus suspension and measured idle/busy rendering cost.
- [Ornament appears to represent actual activity] → Stable labeled actors, clearly
  different contour and route treatments, no invented counters or emotional inference.
- [Preferences unexpectedly beat explicit choices] → Documented precedence and
  process-level tests for config/CLI overrides, provider pairs, resumption and isolation.
- [New layouts hide important controls] → Progressive two-row composer controls,
  tiny-terminal fallback and real resize/edit tests.

## Operational surface

The existing local Rust terminal binary and SQLite store remain the only runtime
surfaces. No new secret, network listener, service, architecture requirement or
dependency is introduced. Start with mise run run; KURU_REDUCED_MOTION=1 remains a
startup accessibility override. The cmux preview will use the same persistent store
across its restart checks; existing user sessions will be preserved.
