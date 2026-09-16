# Terminal design

Kuru gives a changing peer pool a stable visual home: a spacious wordmark, an
open conversation canvas and a composer that holds model, effort and framework
values beside their shortcuts. Project and session context sit in the header.
The status line carries active work, elapsed time, cancellations, failures and
brief saved-setting confirmations. It stays quiet on an untouched welcome screen.

Ink surfaces, mint, blue, lilac, amber and rose distinguish identities and activity.
Color always accompanies readable labels. The ornamental alphabet is ordinary
ASCII; node and control symbols use standard terminal glyphs without an icon font.

## Framework portraits

| Framework | Geometry | Meaningful content |
| --- | --- | --- |
| IFS | Broken concentric orbits | Actual parts in role clusters |
| Polyvagal | Three layered flowing traces | Connection, mobilization, conservation |
| Freudian | Open triangle and interference traces | Desire, reality, standards |
| Jungian | Overlapping rosette contours | Patterns, shadow and shared memory |

Names and contour characters stay fixed. A faint color highlight moves through
the existing lines over 24 seconds, independently of typing; composition does not
trigger decoration or restart the ambient clock. The composer separator is static.
Ornament does not represent sentience, inferred work or invented communication.
Thinking, tools, speaking identities, relationship membership and message endpoints
come from runtime events while work is in progress. A completed response instead
uses its returned turn result: its text appears once with the returned speaker or
relationship and a subdued input/output token line. That line names tool-call and
peer-round limits independently. An empty model response appears as a separate
response outcome rather than a budget, and an older limited result whose cause was
not stored is labeled as an unspecified legacy limit.
Private peer messages and state notes stay out of the activity feed. Numbered
nodes map to the adjacent roster in compact sidebars.

The welcome scene uses the full canvas. During conversation, wide panes show a
smaller scene and the live roster alongside messages; narrow panes preserve chat,
status and composition. The editor expands for wrapped or multiline drafts, then
scrolls while preserving the caret. Model, effort and framework controls wrap into
two or three rows as needed. Very small terminals retain the editable draft.

Pickers filter by typing or paste, mark the current value and scroll the selection
into view. The framework picker previews geometry before committing a change;
previews of other modes show their built-in members and are labeled as previews.
No-match results remain editable. Settings cannot change halfway through active
work; the status line explains that constraint without losing the draft.

Conversation styling distinguishes speakers, headings, quotations, bullets, code
fences, inline code and bold text. This is a small terminal presentation layer,
not a full Markdown engine.

## Motion and rendering

Motion stays on by default: ambient frames are capped at 4 FPS and factual work
indicators at 12.5 FPS. Editing does not accelerate animation. Terminal focus reporting
pauses decoration in background panes. `KURU_REDUCED_MOTION=1` provides a static
startup accessibility override; useful work status and elapsed seconds still
update. Animation never changes the draft or caret.

`apps/kuru-tui/src/ui.rs` maps events and input to view state and a supplied animation
clock. `ui/render.rs` draws layout, text and controls. `ui/scene.rs` draws the four
portraits from that state. Rendering never accesses providers or private memory.
Transcript layout is cached until text or width changes. No dependencies were
added for this presentation system.

## References and review

OMP's [semantic theme colors](https://github.com/can1357/oh-my-pi/blob/main/packages/coding-agent/src/modes/theme/schema.ts),
[animated loader](https://github.com/can1357/oh-my-pi/blob/main/packages/tui/src/components/loader.ts)
and [status line](https://github.com/can1357/oh-my-pi/blob/main/packages/coding-agent/src/tui/status-line.ts)
informed the use of semantic accents and small purposeful motion.
[OpenCode](https://github.com/anomalyco/opencode) and
[Crush](https://github.com/charmbracelet/crush) informed the study of open terminal
layouts and a stronger wordmark. Kuru's portraits and palette are original code.

Export actual rendered terminal cells with:

```sh
KURU_VISUAL_ARTIFACTS=/tmp/kuru-visual mise exec -- cargo test -p kuru --test visual --locked
mise exec -- cargo test -p kuru --test visual --locked frame_cost_profile -- --ignored --nocapture
```

The first command writes HTML and text review artifacts. The second explicitly
runs the machine-dependent frame-cost profile, excluded from timing assertions
in normal tests. See [the verification record](verification.md) for measurements.
