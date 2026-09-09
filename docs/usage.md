# Using Kuru

Run `kuru --provider demo` for the offline terminal interface. Choose the
`codex` provider after `kuru login`, or set `OPENAI_API_KEY` and select
`--provider responses`. The selected model and effort are visible in the
terminal status area; `kuru models` prints the provider's current catalog.

## Terminal controls

| Control | Action |
| --- | --- |
| Enter | Send the input |
| Alt+Enter | Insert a newline |
| F2 / F3 / F4 | Select model / effort / framework |
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
