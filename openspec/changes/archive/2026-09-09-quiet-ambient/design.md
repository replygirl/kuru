## Context

Every editor mutation injected an expanding ring into the portrait and a traveling
line into the composer, then temporarily accelerated drawing. The ambient layer
also swapped punctuation abruptly at a brightness threshold.

## Decisions

Delete input-energy/serial state and reactive drawing rather than tuning bursts.
The existing contour shapes use fixed ASCII glyphs with a smooth, dim 24-second
color cycle. Keep ambient scheduling at 250ms, independent of typing, and preserve
the 80ms cadence for factual busy indicators. The composer stays still. The
successful mode selector behavior and all settings persistence remain intact.

## Risks / Trade-offs

Ambient color is deliberately easy to overlook, especially in low-color terminals.
Shape and label readability take priority; reduced motion and focus pausing retain
existing behavior. Compare actual rendered cells and inspect the live preview.

## Operational surface

Local Rust TUI in the existing terminal process; no new bind address, container,
secret, connection limit, dependency or platform requirement. Existing focused
ambient and busy frame ceilings apply. Live review uses the user's cmux pane.
