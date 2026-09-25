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
| F5 or `/permissions` | Inspect session and saved tool grants; Delete revokes the selected scope |
| 1 / 2 / 3 / 4 (or Alt+1–4) | Answer a tool prompt: once / session / always / deny |
| Type / paste in a picker | Filter the available choices |
| Escape | Close a picker or cancel active work |
| Ctrl+C | Cancel active work; quit while idle |
| Page Up / Page Down | Scroll the transcript |
| Tab / Shift+Tab | Cycle matching slash-command names before their arguments |
| `/help` | Show commands |
| `/clear` | Clear this terminal's visible conversation while retaining stored history and session identity |
| `/new` | Start a new public session using the current project memory |
| `/sessions` | Open the session picker; Enter resumes, Delete removes, Ctrl+R restores, Ctrl+L renames, and Ctrl+F opens settled fork boundaries |
| `/resume SESSION_ID` | Resume one exact active session |
| `/export [PATH]` | Export the current public session as Markdown |
| `/status` | Show local session, project, selections, turns, and known usage without a provider call |
| `/tools` | Inspect filtered tools and disabled/live/stale/degraded MCP aliases without a provider call |
| `/mcp login [--device\|--no-browser] ALIAS` | Sign in to one OAuth-enabled MCP alias |
| `/mcp status ALIAS` | Inspect redacted local MCP authorization and availability state |
| `/mcp logout ALIAS` | Delete the local credential and report remote revocation separately |
| `/parts` | Inspect parts and relationships |
| `/mode ifs` | Change the current framework |
| `/model MODEL_ID` | Select a model |
| `/effort LEVEL` | Set provider reasoning effort |
| `/focus NAME_OR_ID` | Temporarily select the speaking identity |
| `/focus auto` | Return to contextual speaker selection |
| `/relate alliance ID,ID` | Activate a relationship of 2–4 parts |
| `/memory NAME_OR_ID` | Inspect an identity's stored memory |
| `/notes NAME_OR_ID` | Inspect an identity's durable notes, separate from its conversation |
| `/memory-candidates [CURSOR]` | List one bounded page of retained dream candidate refs |
| `/memory-candidate-status BRANCH` | Recheck one exact retained candidate ref |
| `/memory-candidate-abandon BRANCH BASE HEAD` | Explicitly abandon the exact inspected candidate |
| `/memory-status` | Inspect the project's managed memory status |
| `/memory-history` | Inspect committed memory revisions |
| `/cost` | Inspect this session's reported usage and estimated API cost |
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

Tab completion changes only the command name before the first argument; editing
the draft starts a new completion cycle. `/clear` drops rendered conversation rows
from this terminal, not durable turns, usage, or the current session. A resumed
session still has its stored history. `/status` reports the current local view;
it does not ask the provider for an update.

You can add a terminal prompt command as `.kuru/commands/NAME.md` in the project
or `commands/NAME.md` under Kuru's user configuration directory. The Markdown
file begins with YAML `name` and `description` fields between `---` lines;
`name` must match the lowercase file stem. Its remaining text is the prompt.
For example, `.kuru/commands/review.md` registers `/review`. Running
`/review some notes` sends that captured prompt plus the separately labeled,
literal `some notes` as a normal user turn in the current session. Arguments
are optional; the command never runs a shell. `/help` and Tab show the effective
entries after workspace
trust; built-in names win collisions, then project entries, then user entries.
Malformed or oversized entries are omitted with a bounded notice. See
[prompt-source bounds and approval](configuration.md#skills-and-custom-prompts).

`/notes` returns the selected identity's newest 100 durable notes with the
selected mode, canonical identity, requested limit, and `truncated` metadata.
It does not show that identity's conversation; use `/memory` for that history.

## Usage and context

`/cost` includes inference for deliberation, speaking, peer consultation and
dreaming, including reported usage from failed or interrupted work. Cached input
and reasoning output are subsets of the input and output totals. They are not
added a second time. A completed `/retry` does not make a new provider call or add
usage.

Missing reports and prices are shown as unknown or incomplete. An incomplete
estimate names the priced terms it left out, such as a long-context tier or a
cache-write rate, and a term that cannot be decided is never applied in part.
Each record keeps its raw per-kind token components and its recorded price, so a
later reading can apply a term this version does not price. Sessions created
before usage accounting remain labelled as having incomplete historical totals
when resumed. Estimates use the price terms recorded for each invocation; later
catalog changes do not reprice earlier work. For ChatGPT subscription access,
an API-equivalent estimate is a comparison with API prices, not a bill or a
measure of remaining subscription quota.

The context indicator describes one request, preferring the facing speaker when
available. It is not a shared context window for the whole pool. Token estimates
name whether they use local `o200k` tokenization or a byte fallback; neither is
an exact provider count. Matching common prompt prefixes do not establish a cache
hit. Only a provider's reported cached-input tokens count as observed cache use,
and an absent report remains unknown. Assumed window limits and omitted history
are labelled. The composer keeps a
standing row with that context estimate and the session cost estimate, beside the
model, effort, framework and permission controls, so one frame carries all six.
Its figures are the ones `/cost` and `/permissions` report. Earlier conversation
rows omitted from the bounded display remain in storage. See
[context budget](configuration.md#context-budget) for fitting and reserve settings.

## The live interface

Kuru uses an ink background with distinct accents for roles, speakers and activity.
Wide terminals show the peer constellation, current phases and relationship
memberships beside the conversation. Narrow panes prioritize chat and the editor.
The activity feed shows peer routing and phase changes without printing private
peer messages or state notes.

While the selected speaker responds, a provisional preview shows its latest
text, including an unfinished line. A separate thinking region shows a
provider-supplied visible summary or the current activity when no summary is
available. That activity label names what is actually happening: `Responding`
while facing text or a visible summary is streaming, the generic `Calling
tool` while a tool call's arguments are still streaming and the provider has
not yet named it, and `Calling {name}` with the tool's raw catalog name once
the call is dispatched. When the speaker has more than one call in flight at
once, the label names whichever was dispatched most recently; it returns to
`Responding` the moment every one of them has settled, never lagging behind
on a name whose call already finished. Tool arguments are never read,
retained or shown. This also works when a relationship speaks; private
deliberation, peer consultations and dreams remain hidden.

The preview keeps a bounded tail and indicates when earlier text is omitted.
If tools lead to another speaking round, the next draft replaces the previous
one. The completed answer replaces the preview and enters the conversation
once. Cancelling clears the preview; provisional text and visible summaries
are not saved or replayed by resume or `/retry`. Scripted `kuru run` text and
JSON output remain final-only.

An asked tool pauses for an inline permission decision above the composer. The
prompt shows the tool, a redacted preview and the exact grant scope. Ordinary
typing and paste continue to edit your draft; 1–4 or Alt+1–4 answer the
prompt (plain digits work in terminals that swallow the Alt modifier).
Escape or Ctrl+C cancels the active turn without granting the pending tool.
Once covers this invocation; session and always cover the displayed exact file
or whole-tool scope. If that scope cannot be shown completely without redaction,
the prompt offers only once and deny. The permission chip shows remembered grant
counts; F5 or `/permissions` opens their inspection and revocation view.
Use Up/Down to select a grant, PgUp/PgDn to read a long scope, and Delete to
revoke it. Up/Down scrolls long scopes in a pending permission prompt.

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
kuru sessions rename SESSION_ID "New label"
kuru sessions remove SESSION_ID
kuru sessions restore SESSION_ID
kuru sessions fork SESSION_ID SETTLED_NODE_ID --label "New branch"
kuru sessions export SESSION_ID --format jsonl --output transcript.jsonl
kuru sessions export SESSION_ID --format markdown --output transcript.md
kuru --continue --provider demo run "Continue the latest active session."
kuru --resume SESSION_ID --provider demo run "Continue from our last turn."
kuru --resume SESSION_ID --provider demo run "Continue from our last turn." --turn-id TURN_ID
kuru memory notes ID --limit 100
kuru memory candidates --limit 16
kuru memory candidate-status BRANCH
kuru memory candidate-abandon BRANCH --base BASE --head HEAD
```

JSON output includes `session`, `speaker`, `text`, `relationship`, token counts,
`limited`, `limit_reasons`, `response_outcome` and an event trace. New results
name `tool-calls` and `peer-rounds` separately; an empty model response is the
separate `empty` response outcome. `limited` remains for compatibility, and an
older limited record whose specific cause was not stored reports
`legacy-unspecified`. Session listing does not create a new session. `--resume`
selects the exact ID; `--continue` selects the latest nonremoved session by
durable catalog order. Removing a session hides it from ordinary listing and
resume, but preserves its ID, transcript and revision history for restoration.
No session command deletes project memory. A fresh process asks again for
session-only tool grants.

Each public turn has a durable boundary ID. `kuru sessions` reports the latest
settled `head_node_id`, and the terminal session picker lets you choose an older
settled boundary with Ctrl+F and Page Down. Forking copies the public conversation
prefix through that completed or terminally interrupted boundary into a new
session. Pending work cannot be a fork boundary. The fork and its source then
grow independently, while both use current shared project memory; forking does
not rewind private memories or topology. Existing legacy rows retain their
original bytes. Where a historical speaker or turn cannot be proved, exports
label the field as unknown instead of assigning the current part.

`kuru sessions export` writes one public transcript in chronological order,
including typed content, settlement and fork provenance. Markdown is the default;
JSONL has one manifest followed by transcript records. `--output` can replace a
selected existing file after checking its identity; without it, output goes to
stdout. This export omits private actor and relationship histories, reasoning
summaries, notes and candidate branches. Use `kuru memory export` for a complete
memory export. P11 still admits one conversation driver per project; simultaneous
drivers and live-session presence arrive with the separate concurrent-session
work.

The returned event trace freezes with the response. Periodic dreaming runs as
later maintenance, so its activity or failure cannot remove a completed answer.
Exit dreaming has a finite deadline; Kuru still attempts actor and tool cleanup
when the 30-second deadline or the dream itself fails.

Each serialized event remains exactly `{ "kind", "actor", "detail" }`, while
the runtime and terminal consume typed, projected event values. A settled tool
call adds a `tool-observation` event whose detail contains bounded projected
arguments, outcome, admission-to-settlement milliseconds, and receipt metadata
only. It occurs once for each attempted invocation; an optional `tool` start
activity has no outcome or receipt claim and does not add a second settlement.
Call IDs are request-scoped and are never globally deduplicated. `argument_bytes` is the UTF-8 byte length of serialized projected
arguments. `result_bytes` and `result_sha256` describe the same serialized
projected result value, including JSON string quoting and escaping; a call with
no result has zero result bytes and no digest. Completed journals now write
format 2 and replay older format-1 event triplets through their legacy
projector without rewriting historical revisions.

`kuru memory notes ID` reads an existing project's selected-mode durable notes
without starting a conversation, provider, or tool. It returns the same bounded
notes view as `/notes`: `mode`, canonical `identity`, chronological `notes`,
`requested_limit`, and `truncated`. Limits are 1 through 1000 and default to
100. An exact retained part or relationship ID remains readable after it is no
longer active; names and roles resolve only among active identities.

`kuru memory candidates` and `/memory-candidates` inspect bounded pages of
retained dream candidate refs without changing them. Pass the returned opaque
cursor to `--after` or as the TUI command argument. Recheck one selected branch
with `candidate-status` before abandonment. A missing ref is reported with an
unproved operation outcome; closing and reopening does not turn absence into
proof. `candidate-abandon` requires the exact inspected branch, base, and head,
then rechecks them under the memory owner before changing the ref. Changed,
active, stale, or uncertain refs remain intact. These commands never promote,
merge, replay, or automatically abandon a candidate.

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
kuru -C /path/to/project --allow-write tool file_edit \
  --args '{"path":"notes.txt","hunks":[{"before":"A ","old":"working","after":" note.","replacement":"revised"}]}'
```

`tools` and the TUI's `/tools` command inspect one shared catalog: built-ins,
filtered MCP metadata, and each configured alias's `disabled`, `live`, `stale`,
or `degraded` state. Stale metadata is descriptive only and cannot be called.
Use a returned stable namespaced identifier from a live alias when calling an
MCP tool. Discovery and calls fail clearly when a configured server is unavailable. Ask-capable tools remain
discoverable. Direct CLI calls use the same rules and saved grants as the TUI,
but an unresolved ask refuses immediately instead of opening a prompt. Shell
uses your process permissions; it is not a sandbox. See
[tool permissions](configuration.md#tool-permissions) for legacy flags, explicit
rules, grant lifetimes and deny precedence.

Native `file_write`, `file_edit`, and `file_delete` keep private, project-bound
before/after checkpoints outside the tool root. `file_edit` applies 1–64 ordered
exact-context hunks to one UTF-8 source capture; stale, ambiguous, overlapping,
or oversized edits change nothing. Each snapshot is capped at 2 MiB and the
project checkpoint inventory at 128 MiB (including conservative retained-stage
reservations) and 10,000 private inventory entries. A full inventory refuses another file
mutation before touching its target; checkpoints do not expire automatically.
Checkpointed writes to multiply linked files are refused before any effect.
Earlier `file_write` could replace just the selected pathname, but capturing
the required before snapshot could read an out-of-root alias that `file_read`
already refuses.
Pruning a settled receipt also forfeits its selected undo and exact-retry
evidence.
Shell and MCP effects are outside file checkpoint and undo coverage.

```sh
kuru -C /path/to/project file list
kuru -C /path/to/project file inspect CHECKPOINT_ID
kuru -C /path/to/project --allow-write file undo CHECKPOINT_ID
kuru -C /path/to/project file prune CHECKPOINT_ID
kuru -C /path/to/project file prune CHECKPOINT_ID --discard-uncertain
```

The TUI has `/file-checkpoints`, `/file-inspect ID`, `/file-undo ID`, and
`/file-prune ID [--discard-uncertain]`. Inspection shows IDs, paths, effect and
status without snapshot bodies. A durably applied checkpoint can be undone
after restart if the checked current file still matches its recorded post-state;
undo performs a new exact-file permission check and records its own effect.
File undo is separate from `undo-dream` and never rewinds conversation history.
If a process stops before the applied marker is durable, matching file bytes
or an absent deleted path cannot prove who made the effect. The checkpoint stays
unresolved and cannot be retried or undone automatically. Explicitly discarding
one inactive unresolved receipt forfeits its recovery and undo evidence without
changing the project file. Discard removes only a stage and empty access template
whose captured identities still match; if the target parent or those artifacts
were replaced, it refuses and retains the receipt for manual directory recovery.
A later target with a different checked identity or
bytes causes undo to conflict instead of overwriting it.

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
prints the resolved ring directory once on standard error. That detail includes
the shape of each reconciled provider stream — every streamed and terminal
output item's ID, kind, position and text length, never its text. It does not enable
`RUST_LOG`, capture prompts, tool arguments/results, credentials, or remote error
text, and it does not change command stdout or TUI rendering. These files are
operational diagnostics, not conversation history or semantic turn events.
For native provider requests, debug records pair the local input-token estimate
and final body byte length with the provider's reported input and cached subset;
they do not record the request body or imply that a matching prefix was cached.
