# Using Kuru

Run `kuru --provider demo` to explore the terminal without provider credentials.
The executable includes the verified Dolt engine, so the first demo conversation
works offline with an empty cache. See [memory storage](memory.md) for local
extraction and storage details. Choose the
`codex` provider after `kuru login`, or set `OPENAI_API_KEY` and select
`--provider responses`. The selected model and effort are visible in the
composer beside their shortcuts; `kuru models` prints the provider's current catalog.

## Terminal controls

| Control | Action |
| --- | --- |
| Enter | Send the input |
| Alt+Enter | Insert a newline |
| F2 / F3 / F4 | Select model / effort / framework |
| Type / paste in a picker | Filter the available choices |
| Escape | Close a picker or cancel active work |
| Ctrl+C | Cancel active work; quit while idle |
| Page Up / Page Down | Scroll the transcript |
| `/help` | Show commands |
| `/parts` | Inspect parts and relationships |
| `/mode ifs` | Change the current framework |
| `/model MODEL_ID` | Select a model |
| `/effort LEVEL` | Set provider reasoning effort |
| `/focus NAME_OR_ID` | Temporarily select the speaking identity |
| `/focus auto` | Return to contextual speaker selection |
| `/relate alliance ID,ID` | Activate a relationship of 2–4 parts |
| `/memory NAME_OR_ID` | Inspect an identity's stored memory |
| `/dream` | Run bounded memory/topology consolidation |
| `/undo-dream` | Restore the previous topology change |
| `/quit` | End the session |

Supported relationship kinds are `protection`, `polarization` and `alliance`.
Use `/parts` to find unambiguous identities; roles containing multiple members
need a unique name or ID. The human user can inspect private memories; parts
receive only the memory available to their own identity.

## The live interface

Kuru uses an ink background with distinct accents for roles, speakers and activity.
Wide terminals show the peer constellation, current phases and relationship
memberships beside the conversation. Narrow panes prioritize chat and the editor.
The activity feed shows peer routing and phase changes without printing private
peer messages or state notes.

Each framework has its own portrait: IFS orbits, polyvagal flows, a Freudian
triangle and a Jungian rosette. Quiet ASCII contours animate while the pane has
focus, at up to four frames per second. A faint color highlight moves through
fixed contour characters over 24 seconds. Typing leaves that rhythm and the composer
decoration alone. Active work indicators can update at up to 12.5 frames per second.
Motion stays on by default.
`KURU_REDUCED_MOTION=1 kuru` is an accessibility startup override that makes
ornament static. The actual operation timer still updates. No special icon font
is required.

F2, F3 and F4 display the current value beside its shortcut in the composer.
Pickers accept typed or pasted filters and keep your unsent draft. F4 previews
the selected framework in wider panes; Enter applies it. The effort picker always
includes `default`, which clears an explicit effort selection. Choices are saved
for the current project and survive quit/relaunch. A new launch opens a new
conversation; `--resume` restores an existing one. See [configuration precedence](configuration.md)
for how temporary command-line and explicit configuration overrides interact with
remembered choices.

See [terminal design](interface.md) for the visual system and its implementation.

## Scripted conversations and sessions

```sh
kuru --provider demo --mode jungian run "Explore the assumptions in this design."
kuru --provider demo run "Plan the next step." --json
kuru sessions
kuru --resume SESSION_ID --provider demo run "Continue from our last turn."
```

JSON output includes `session`, `speaker`, `text`, `relationship`, token counts,
`limited` and an event trace. `limited` reports that a round or tool budget
constrained the turn. Session listing does not create a new session. Reusing a
session restores its transcript; peer and relationship histories also persist
across sessions within their project and framework scope.

`kuru --resume SESSION_ID dream` runs explicit consolidation and
`kuru --resume SESSION_ID undo-dream` restores the previous accepted topology.
Periodic dreaming follows `dream_every`; session-end dreaming follows
`dream_on_exit`. `--no-dream` disables automatic dreaming for that invocation.
The CLI prints a completed answer before performing any configured exit dream.

## Workspace tools

```sh
kuru -C /path/to/project tools
kuru -C /path/to/project tool file_read --args '{"path":"README.md"}'
kuru -C /path/to/project --allow-write tool file_write \
  --args '{"path":"notes.txt","content":"A working note."}'
```

`tools` discovers built-ins and configured MCP tools. Use the returned stable
namespaced identifier when calling an MCP tool. Discovery and calls fail
clearly when a configured server is unavailable. Shell requires
`--allow-shell` and runs with your process permissions; it is not a sandbox.
See [configuration](configuration.md) for persistent permissions and budgets.

## Authentication and service commands

`login`, `login --device`, `auth` and `logout` use supported Codex commands.
`config` prints merged configuration, redacting MCP environment values.
`serve` exposes the authenticated local A2A subset; see [protocols](protocols.md).
`update` installs an explicit verified release or rebuilds a chosen source
checkout; see [installation](install.md).
