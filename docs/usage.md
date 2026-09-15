# Using Kuru

Run `kuru --provider demo` to explore the terminal without provider credentials.
The executable includes the verified Dolt engine, so the first demo conversation
works offline with an empty cache. See [memory storage](memory.md) for local
extraction and storage details. Choose the
default `codex` provider after Kuru's own ChatGPT sign-in with `kuru login`, or
set `OPENAI_API_KEY` and select `--provider responses --model MODEL_ID`.
The selected model and effort are visible in the
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
| `/notes NAME_OR_ID` | Inspect an identity's durable notes, separate from its conversation |
| `/retry` | Retry the last local submission when its exact durable turn is safe to reuse |
| `/dream` | Run bounded memory/topology consolidation |
| `/undo-dream` | Restore the previous topology change |
| `/quit` | End the session |

Supported relationship kinds are `protection`, `polarization` and `alliance`.
Use `/parts` to find unambiguous identities; roles containing multiple members
need a unique name or ID. The human user can inspect private memories; parts
receive only the memory available to their own identity.

Cancelling active work signals the runtime and waits for the operation to settle.
The submitted prompt remains in the conversation. If the answer checkpoint won
the race, Kuru displays that answer. Otherwise it adds the fixed marker `Turn
interrupted; no completed answer was committed.` The marker survives later turns
and session resume without claiming that external work was absent or rolled back.

`/retry` addresses only the last durably retained local submission. It reuses the
same turn ID, exact prompt and target. A completed turn returns its stored result
without another provider or tool call and without adding another displayed pair.
An interruption before possible dispatch can finish safely under the same ID;
its earlier marker remains visible. If external dispatch may have occurred, Kuru
refuses the retry and explains that entering a new prompt starts new work. Kuru
never retries automatically and does not provide an arbitrary turn-history
browser.

`/notes` returns the selected identity's newest 100 durable notes with the
selected mode, canonical identity, requested limit, and `truncated` metadata.
It does not show that identity's conversation; use `/memory` for that history.

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
kuru --resume SESSION_ID --provider demo run "Continue from our last turn." --turn-id TURN_ID
kuru memory notes ID --limit 100
```

JSON output includes `session`, `speaker`, `text`, `relationship`, token counts,
`limited`, `limit_reasons`, `response_outcome` and an event trace. New results
name `tool-calls` and `peer-rounds` separately; an empty model response is the
separate `empty` response outcome. `limited` remains for compatibility, and an
older limited record whose specific cause was not stored reports
`legacy-unspecified`. Session listing does not create a new session. Reusing a
session restores its transcript; peer and relationship histories also persist
across sessions within their project and framework scope.

The returned event trace freezes with the response. Periodic dreaming runs as
later maintenance, so its activity or failure cannot remove a completed answer.
Exit dreaming has a finite deadline; Kuru still attempts actor and tool cleanup
when the 30-second deadline or the dream itself fails.

`kuru memory notes ID` reads an existing project's selected-mode durable notes
without starting a conversation, provider, or tool. It returns the same bounded
notes view as `/notes`: `mode`, canonical `identity`, chronological `notes`,
`requested_limit`, and `truncated`. Limits are 1 through 1000 and default to
100. An exact retained part or relationship ID remains readable after it is no
longer active; names and roles resolve only among active identities.

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

`login` starts browser sign-in for ChatGPT subscription access.
`login --no-browser` prints the URL for you to open; `login --device` uses device
authorization. `auth` prints redacted local authentication status as JSON, and
`logout` clears Kuru's ChatGPT credentials. These commands manage Kuru's own
private auth store without a Codex executable or another application's tokens.
See [authentication configuration](configuration.md#authentication) for storage
and API-key provider selection.
`config` prints configured values and CLI overrides, redacting MCP environment
values and omitting saved project selections without opening memory. Automatic
ancestor configuration that grants process, mutation, executable, credential-route
or endpoint authority is reviewed for the exact canonical workspace before it is
activated. Use `trust status`, `trust approve`, `trust revoke`, or the
non-persistent `--trust-workspace-once` flag; see
[workspace trust](configuration.md#workspace-trust) for command-specific behavior
and its process-authority limits.
`serve` exposes the authenticated local A2A subset; see [protocols](protocols.md).
`update` installs an explicit verified release or rebuilds a chosen source
checkout; see [installation](install.md).

Runtime-owning commands keep a private per-project operational ring at
`<data-dir>/diagnostics/<project-hash>/trace-{0..3}.jsonl`: four files of at most
64 KiB each. `--debug` adds bounded operational status detail to those files and
prints the resolved ring directory once on standard error. It does not enable
`RUST_LOG`, capture prompts, tool arguments/results, credentials, or remote error
text, and it does not change command stdout or TUI rendering. These files are
operational diagnostics, not conversation history or semantic turn events.
