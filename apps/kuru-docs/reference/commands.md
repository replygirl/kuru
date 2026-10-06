# Commands & shortcuts

Run `kuru --help` for the installed CLI's options, or `/help` inside the terminal. With no subcommand, `kuru` opens the chat interface.

## Terminal keys

| Key                                              | Action                                                                           |
| ------------------------------------------------ | -------------------------------------------------------------------------------- |
| <kbd>Enter</kbd>                                 | Send the input or select the highlighted picker item                             |
| <kbd>Alt</kbd>+<kbd>Enter</kbd>                  | Insert a newline                                                                 |
| <kbd>Left</kbd> / <kbd>Right</kbd>               | Move one displayed character (grapheme)                                          |
| <kbd>Up</kbd> / <kbd>Down</kbd>                  | Move vertically; at the first/last line, browse this session's submitted prompts |
| <kbd>Ctrl</kbd>+<kbd>R</kbd>                     | Search this session's in-memory submitted prompts; repeat to cycle matches       |
| <kbd>Escape</kbd> during prompt search           | Restore the unsent draft                                                         |
| <kbd>Ctrl</kbd>+<kbd>G</kbd> on a paste chip     | Expand or compact its display                                                    |
| <kbd>Ctrl</kbd>+<kbd>X</kbd> on a paste chip     | Remove exactly the pasted text                                                   |
| <kbd>F6</kbd>                                    | Select the next tool card observed in this session                               |
| <kbd>F7</kbd>                                    | Expand or collapse the selected tool card                                        |
| <kbd>F2</kbd>                                    | Choose a model                                                                   |
| <kbd>F3</kbd>                                    | Choose reasoning effort                                                          |
| <kbd>F4</kbd>                                    | Choose a framework                                                               |
| Type or paste in a picker                        | Filter its choices                                                               |
| <kbd>Escape</kbd>                                | Close a picker or cancel active work                                             |
| <kbd>Ctrl</kbd>+<kbd>C</kbd>                     | Cancel work; quit while idle                                                     |
| <kbd>Page Up</kbd> / <kbd>Page Down</kbd>        | Scroll the transcript                                                            |
| <kbd>Tab</kbd> / <kbd>Shift</kbd>+<kbd>Tab</kbd> | Cycle matching slash-command names before the first argument                     |

Prompt history is bounded and held only in memory for the current run. It is
separate from durable conversation history and is grouped by session. Pasted
text larger than 512 bytes or containing at least three line breaks appears as
a compact chip; expand it to inspect the text or remove the exact pasted span.
The composer accepts at most 128 KiB of draft text. An oversized paste is
rejected in full, leaving the draft unchanged.

## Slash commands

| Command                                      | Action                                                                                   |
| -------------------------------------------- | ---------------------------------------------------------------------------------------- |
| `/help`                                      | Show terminal help                                                                       |
| `/clear`                                     | Clear the visible conversation; keep stored turns and the session                        |
| `/new`                                       | Start another session using current shared project memory                                |
| `/sessions`                                  | Open the session picker for resume, rename, remove, restore and fork                     |
| `/resume SESSION_ID`                         | Resume one exact active session                                                          |
| `/export [PATH]`                             | Export this public session as Markdown                                                   |
| `/status`                                    | Show the current session, project, selections, turns, and known usage locally            |
| `/config`                                    | Inspect the captured effective configuration, source layers, and leaf provenance locally |
| `/compact [ID]`                              | Compact retained context for one or all active identities                                |
| `/parts`                                     | Inspect active parts and relationships                                                   |
| `/mode ifs`                                  | Select `ifs`, `polyvagal`, `freudian`, or `jungian`                                      |
| `/model MODEL_ID`                            | Select a model and its advertised default effort                                         |
| `/effort LEVEL`                              | Select effort; `default` clears an explicit value                                        |
| `/focus NAME_OR_ID`                          | Select a speaking identity                                                               |
| `/focus auto`                                | Return to contextual speaker selection                                                   |
| `/relate KIND ID,ID`                         | Activate protection, polarization, or alliance among 2–4 members                         |
| `/memory NAME_OR_ID`                         | Inspect an identity's stored memory                                                      |
| `/notes NAME_OR_ID`                          | Inspect an identity's separate bounded durable notes                                     |
| `/memory-candidates [CURSOR]`                | List one bounded page of retained dream candidate refs                                   |
| `/memory-candidate-status BRANCH`            | Recheck one exact retained candidate ref                                                 |
| `/memory-candidate-abandon BRANCH BASE HEAD` | Explicitly abandon the exact inspected candidate                                         |
| `/retry`                                     | Safely retry the last durably retained local submission                                  |
| `/cost`                                      | Show this session's reported usage and estimated API cost                                |
| `/permissions`                               | Inspect session and saved tool grants                                                    |
| `/tools`                                     | Inspect built-in and effective MCP tool aliases and their current status                 |
| `/memory-status`                             | Inspect the project's memory store and current revision                                  |
| `/memory-history`                            | List committed memory revisions                                                          |
| `/dream`                                     | Run bounded consolidation                                                                |
| `/undo-dream`                                | Restore the previous accepted topology change                                            |
| `/quit`                                      | End the session                                                                          |

Model, effort, and framework selections made here are [saved for the project](./configuration#remembered-choices). Identity names must be unambiguous; `/parts` provides IDs.

`/config` is read-only and uses the configuration snapshot captured when the TUI started; it does not reread configuration files or activate new workspace authority. It shows bounded per-layer and final-leaf sources, redacts secret-bearing values, and leaves named environment-variable references visible without resolving them. The captured saved mode/model/effort and the current live runtime selection are labeled separately. `/config` takes no arguments and does not open an editor.

`/compact` reports the actor, covered source range and accepted summary identity; automatic compaction shows the same checkpoint information. These notices contain no private summary or history text, and the original records remain stored. Accepted checkpoints still produce a notice if the surrounding operation is cancelled. A candidate checkpoint notice does not mean the candidate was promoted. Headless `kuru run` writes compaction notices to stderr, preserving stdout's answer or JSON.

Ctrl-C cancels an active headless run and waits for its operation and memory cleanup. Confirmed compaction notices remain on stderr. Plain/final JSON output begins after a completed turn; opt-in stream JSON can already contain public progress. Interrupting output never undoes a completed turn.

Tab completes only the leading command name and leaves its arguments unchanged. `/clear` affects only the current terminal view: stored conversation, usage, and session identity remain available, including after resume. `/status` uses the local session snapshot and makes no provider request.

Custom prompt commands use `.kuru/commands/NAME.md` in the project or `commands/NAME.md` under Kuru's user configuration directory. A file needs YAML `name` and `description` frontmatter followed by a nonempty Markdown prompt. `/NAME` sends the captured prompt as a normal turn; optional arguments are separately labeled literal text. It never runs a shell command. Built-in names win collisions, followed by project and user entries. After workspace trust, `/help` and Tab show only effective commands. Project entries join the complete base authority manifest; see [skills and custom prompts](./configuration#skills-and-custom-prompts).

## CLI commands

| Command                                                        | Purpose                                                                                             |
| -------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- |
| `kuru run ["PROMPT"]`                                          | Run one turn without the TUI; accept piped UTF-8 input and add `--json` for final structured output |
| `kuru run "PROMPT" --turn-id ID`                               | Use a bounded explicit turn ID for exact retry in the resumed session                               |
| `kuru login`                                                   | Start browser sign-in for ChatGPT subscription access                                               |
| `kuru login --no-browser`                                      | Print the browser sign-in URL for you to open                                                       |
| `kuru login --device`                                          | Use device authorization                                                                            |
| `kuru auth`                                                    | Print redacted local authentication status as JSON                                                  |
| `kuru logout`                                                  | Clear Kuru's stored ChatGPT credentials                                                             |
| `kuru models`                                                  | Discover provider models and advertised efforts                                                     |
| `kuru config`                                                  | Print captured base configuration with secret values redacted and saved preferences omitted         |
| `kuru trust status`                                            | Inspect exact-workspace authority and complete-manifest approval without creating state             |
| `kuru trust approve [--yes]`                                   | Review and persist approval for the complete current authority manifest                             |
| `kuru trust revoke`                                            | Remove the exact workspace's approval without confirmation                                          |
| `kuru sessions`                                                | List saved sessions                                                                                 |
| `kuru sessions rename ID LABEL`                                | Change one session's label                                                                          |
| `kuru sessions remove ID`                                      | Reversibly remove a session from ordinary listing and resume                                        |
| `kuru sessions restore ID`                                     | Restore one removed session with its identity and transcript                                        |
| `kuru sessions fork ID SETTLED_NODE_ID`                        | Fork the public prefix through one settled boundary, sharing current project memory                 |
| `kuru sessions export ID --format jsonl --output PATH`         | Export one public transcript as chronological JSONL; Markdown is also supported                     |
| `kuru memory status`                                           | Inspect the project store, branch and revision                                                      |
| `kuru memory history`                                          | List committed memory updates; use `--limit` to select 1–1000 entries                               |
| `kuru memory candidates`                                       | List retained dream candidate refs; use `--limit` and the opaque `--after` cursor                   |
| `kuru memory candidate-status BRANCH`                          | Recheck one exact retained candidate ref                                                            |
| `kuru memory candidate-abandon BRANCH --base BASE --head HEAD` | Explicitly abandon the exact inspected candidate                                                    |
| `kuru memory notes ID`                                         | Read newest durable notes for one selected-mode identity; `--limit` is 1–1000 (default 100)         |
| `kuru memory forget ID --note SEQUENCE`                        | Remove one selected current note and retain prior revision history                                  |
| `kuru memory purge --yes`                                      | Remove one project's managed current memory and Dolt history after explicit confirmation            |
| `kuru memory export`                                           | Export every application record from one committed active-memory snapshot                           |
| `kuru dream`                                                   | Run explicit consolidation                                                                          |
| `kuru undo-dream`                                              | Restore the previous topology change                                                                |
| `kuru tools`                                                   | Inspect filtered tools and disabled/live/stale/degraded MCP aliases                                 |
| `kuru mcp login ALIAS`                                         | Sign in to one OAuth-enabled MCP alias with a loopback browser callback                             |
| `kuru mcp login ALIAS --no-browser`                            | Print browser and same-host/forwarded-callback guidance                                             |
| `kuru mcp login ALIAS --device`                                | Use an advertised device authorization flow                                                         |
| `kuru mcp status ALIAS`                                        | Inspect redacted local MCP authorization and availability state                                     |
| `kuru mcp logout ALIAS`                                        | Delete the local MCP credential and report remote revocation separately                             |
| `kuru tool NAME --args '{}'`                                   | Invoke a tool with JSON arguments                                                                   |
| `kuru serve`                                                   | Start authenticated loopback [A2A ingress](./a2a)                                                   |
| `kuru update`                                                  | [Install an explicit release or source checkout](/guide/installation#update-deliberately)           |

Supply `--resume SESSION_ID` with `dream` or `undo-dream` when targeting a saved conversation.

Authentication commands use Kuru's private store, without a Codex CLI or another
application's tokens. `auth` does not create credentials or open project memory;
it is not a live access check. `logout` leaves environment-supplied API keys
unchanged. For API-key access, set `OPENAI_API_KEY` and select
`--provider responses --model MODEL_ID`. See [authentication](/guide/authentication).

## Headless input and output

`kuru run` executes one turn. It does not start a batch loop or a control server.
With terminal stdin, supply a prompt argument; Kuru does not wait for terminal
input. With redirected stdin, it reads UTF-8 up to the 128 KiB aggregate prompt
limit before activating configuration, tools, providers or memory. Pipe-only
input is the prompt. An empty pipe preserves the argument exactly; a nonempty
pipe follows the argument after `Piped input (literal data):`. Bare `kuru` still
requires a terminal and does not turn piped input into a headless request.

```sh
printf 'literal source text' | kuru run 'Summarize this source' --json
printf 'Explain this text' | kuru run --output-format stream-json
```

`--json` keeps the final `TurnOutput` object. `--output-format stream-json` is
mutually exclusive with it and writes JSONL envelopes with `version: 1`,
increasing `seq`, `kind`, a safe `turn` correlation and an allowlisted `detail`.

| Kind       | Public content                                                                                         |
| ---------- | ------------------------------------------------------------------------------------------------------ |
| `started`  | Session identity for this one request                                                                  |
| `status`   | Fixed phase or actual tool started/settled status, without arguments/results                           |
| `snapshot` | Facing text, request round, snapshot sequence and truncation; a replaceable snapshot, not a text delta |
| `gap`      | Exact broadcast omissions, observed coalescing and separately unknown watch loss                       |
| `terminal` | Outcome/settlement and completed answer metadata, truncation and exact-retry reuse when available      |

Records are bounded to 256 KiB; stream answer text is bounded to 32 KiB and
reports truncation. Private summaries, peer messages, tool bodies and raw errors
are excluded. These stream caps do not truncate final `--json`, which keeps the
existing complete TurnOutput object and existing turn/provider limits. A signal
during the configured exit dream cancels that same shutdown and awaits cleanup;
it does not cancel an already completed turn. Debug information, hook reports, confirmed compaction notices and
errors remain on stderr. Progress gaps do not report an exact number of lost
tokens. A writable stream receives at most one terminal record after cleanup;
closed output or Ctrl-C during held output can leave partial records or no
terminal. Cancellation delivery is best effort and never delays owned cleanup.

| Exit  | Meaning                                                                                |
| ----- | -------------------------------------------------------------------------------------- |
| `0`   | Completed turn and acknowledged final output; nonfatal denied tools can still complete |
| `2`   | Invalid input/CLI or exact turn ID reused with different input                         |
| `3`   | Overall typed permission or workspace-trust refusal                                    |
| `130` | Controlled cancellation or interruption of held output                                 |
| `141` | Broken stdout after awaited owned cleanup                                              |
| `1`   | Other failure, uncertainty or cleanup failure                                          |

An output error does not undo an accepted effect or completed turn. Resume the
same session with the same `--turn-id` and exact composed input to recover an
already completed result without replay. Kuru refuses ambiguous retries after
possibly dispatched work; it does not retry automatically. If completion wins
Ctrl-C, its accepted result remains completed, even when later output is
interrupted.

## Global options

| Option                   | Effect                                                                     |
| ------------------------ | -------------------------------------------------------------------------- |
| `-C`, `--directory PATH` | Select the project directory                                               |
| `--config PATH`          | Add a final configuration-file layer                                       |
| `--data-dir PATH`        | Choose a separate private storage directory                                |
| `--debug`                | Add bounded local operational detail for runtime-owning commands           |
| `--provider NAME`        | Choose `codex`, `responses`, or `demo`                                     |
| `--mode NAME`            | Choose a framework                                                         |
| `--model ID`             | Choose a model                                                             |
| `--effort LEVEL`         | Set reasoning effort                                                       |
| `--resume ID`            | Resume a saved conversation                                                |
| `--continue`             | Select the latest nonremoved saved conversation by durable catalog order   |
| `--allow-write`          | Permit built-in file mutation tools                                        |
| `--allow-shell`          | Permit shell processes with your account's authority                       |
| `--trust-workspace-once` | Approve only this command's applicable workspace claims without persisting |
| `--no-dream`             | Disable periodic and session-end dreaming for this invocation              |

Global options can be used alongside subcommands. Use `kuru COMMAND --help` for command-specific arguments.

`--turn-id` belongs only to `run`. Pair it with `--resume SESSION_ID` when
retrying a prior scripted turn. The ID never derives from prompt text: repeating
a prompt without the original ID is new work. An exact completed retry returns
the stored result without provider or tool work; a changed request conflicts,
and a turn that may have dispatched external work refuses replay. In the TUI,
`/retry` uses only the one last local submission retained for that session. It
does not browse older turns or enable automatic retry.

For runtime-owning commands, `--debug` prints one JSON line naming the resolved
diagnostic ring directory on standard error. The command result on standard
output is unchanged, including for `run --json`. The directory contains the
four-file, 64 KiB-per-file operational ring
`trace-{0..3}.jsonl`; it is separate from durable turn history. Its records
include the shape of each reconciled provider stream — every streamed and
terminal output item's ID, kind, position and text length, never its text.
A turn's tool call refused before dispatch, because its name was not offered or
a pre-tool hook denied it, leaves one status record that omits the tool name,
arguments and any hook reason.

`/memory` remains conversation inspection. `/notes` and `kuru memory notes ID`
return a JSON view with the selected `mode`, canonical `identity`, chronological
`notes`, `requested_limit`, and `truncated`. Exact retained part or relationship
IDs remain readable; names and roles must resolve among active identities. The
CLI command reads only an existing current store and does not start a provider,
tool host, or conversation. Each note includes its stable `sequence` and stored
role. `kuru memory forget ID --note SEQUENCE` deletes exactly that current notes
row in a new Dolt revision; it leaves conversations, other notes, and earlier
revisions intact, so it is not secure erasure or a history-recovery command.

Candidate inventory and status are read-only and remain available when a prior
dream transition needs explicit resolution. A missing candidate reports its
operation outcome as unproved; reopening does not settle it. Explicit abandon
requires the exact inspected branch, base, and head and rechecks them under the
memory owner. Changed, active, stale, or uncertain refs remain intact. No
candidate command promotes, merges, retries, or automatically abandons work.

`kuru memory purge --yes` explicitly removes the selected canonical project's
managed current Dolt store, all of its managed revisions, and recognised managed
recovery copies. It retains original/shared legacy SQLite input and migration
snapshots, user exports and backups, other projects, engine cache, and stable
lock objects. It suppresses future automatic import of the selected project from
that shared legacy source. It refuses an active owner, never kills it, does not
rewrite history outside Kuru's managed store, and is not secure erasure. After
memory removal it removes the selected bounded diagnostics ring. If an earlier
purge recorded incomplete work, rerun the same command; it only resumes the
recorded remaining identities.

`kuru memory export [--format json|markdown] [--output PATH]` is provider-free
inspection of the current committed `main` snapshot. Its JSON manifest includes
revision, schema and row counts; Markdown contains the same manifest and each
message, state, context-summary and context-cursor record as JSON. Message
records retain an optional physical session identity, including null for
unattributed legacy rows. It excludes previous revisions, candidate branches, uncommitted
working rows, operations and schema tables. A supplied output path is published
only as a new file after private staging; it never overwrites an existing path.
It does not create memory or import legacy SQLite data for a fresh project.
