# Terminal design

Kuru's interface gives the peer pool a visible identity while keeping conversation
and composition central. Ink backgrounds and quiet borders separate surfaces;
mint, blue, lilac, amber and rose distinguish identities and activity. Labels
accompany color, and ordinary terminal glyphs work without a special icon font.

The welcome screen introduces the current framework and its real members. During
a conversation, the pool shows thinking, tool use and speaking states from runtime
events. Relationships show their members, and peer exchanges show their endpoints.
The display never invents progress percentages or prints private peer-message
contents. Narrow panes retain the editor and conversation with a compact pool strip.

Conversation styling distinguishes speakers, headings, quotations, bullets, code
fences, inline code and bold text. This is a small terminal presentation layer,
not a full Markdown engine. Selectors highlight both the current option and the
keyboard selection. All earlier keyboard controls remain available.

Motion is limited to visual accents, activity indicators and a four-second welcome
sequence. Animation uses an 80ms frame clock, with redraws driven by changes when
settled. F6 toggles reduced motion; `KURU_REDUCED_MOTION=1` starts with it enabled.
This option changes presentation only.

The implementation stays inside `apps/tui`: `ui.rs` maps runtime events and input
to view state, while `ui/render.rs` renders that state without accessing providers,
memories or clocks. No new dependencies are needed for the visual system.

Design references include OMP's [semantic theme colors](https://github.com/can1357/oh-my-pi/blob/main/packages/coding-agent/src/modes/theme/schema.ts)
and [bounded animated loader](https://github.com/can1357/oh-my-pi/blob/main/packages/tui/src/components/loader.ts).
Kuru's palette and peer presentation are its own implementation.
