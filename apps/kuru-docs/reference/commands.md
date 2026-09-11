# Commands & shortcuts

Run `kuru --help` for the installed CLI's options, or `/help` inside the terminal. With no subcommand, `kuru` opens the chat interface.

## Terminal keys

| Key                                       | Action                                               |
| ----------------------------------------- | ---------------------------------------------------- |
| <kbd>Enter</kbd>                          | Send the input or select the highlighted picker item |
| <kbd>Alt</kbd>+<kbd>Enter</kbd>           | Insert a newline                                     |
| <kbd>F2</kbd>                             | Choose a model                                       |
| <kbd>F3</kbd>                             | Choose reasoning effort                              |
| <kbd>F4</kbd>                             | Choose a framework                                   |
| Type or paste in a picker                 | Filter its choices                                   |
| <kbd>Escape</kbd>                         | Close a picker or cancel active work                 |
| <kbd>Ctrl</kbd>+<kbd>C</kbd>              | Cancel work; quit while idle                         |
| <kbd>Page Up</kbd> / <kbd>Page Down</kbd> | Scroll the transcript                                |

## Slash commands

| Command              | Action                                                           |
| -------------------- | ---------------------------------------------------------------- |
| `/help`              | Show terminal help                                               |
| `/parts`             | Inspect active parts and relationships                           |
| `/mode ifs`          | Select `ifs`, `polyvagal`, `freudian`, or `jungian`              |
| `/model MODEL_ID`    | Select a model and its advertised default effort                 |
| `/effort LEVEL`      | Select effort; `default` clears an explicit value                |
| `/focus NAME_OR_ID`  | Select a speaking identity                                       |
| `/focus auto`        | Return to contextual speaker selection                           |
| `/relate KIND ID,ID` | Activate protection, polarization, or alliance among 2–4 members |
| `/memory NAME_OR_ID` | Inspect an identity's stored memory                              |
| `/memory-status`     | Inspect the project's memory store and current revision          |
| `/memory-history`    | List committed memory revisions                                  |
| `/dream`             | Run bounded consolidation                                        |
| `/undo-dream`        | Restore the previous accepted topology change                    |
| `/quit`              | End the session                                                  |

Model, effort, and framework selections made here are [saved for the project](./configuration#remembered-choices). Identity names must be unambiguous; `/parts` provides IDs.

## CLI commands

| Command                      | Purpose                                                                                   |
| ---------------------------- | ----------------------------------------------------------------------------------------- |
| `kuru run "PROMPT"`          | Run a turn without the TUI; add `--json` for structured output                            |
| `kuru login`                 | Start browser sign-in for ChatGPT subscription access                                     |
| `kuru login --no-browser`    | Print the browser sign-in URL for you to open                                             |
| `kuru login --device`        | Use device authorization                                                                  |
| `kuru auth`                  | Print redacted local authentication status as JSON                                        |
| `kuru logout`                | Clear Kuru's stored ChatGPT credentials                                                   |
| `kuru models`                | Discover provider models and advertised efforts                                           |
| `kuru config`                | Print merged configuration with MCP environment values redacted                           |
| `kuru sessions`              | List saved sessions                                                                       |
| `kuru memory status`         | Inspect the project store, branch and revision                                            |
| `kuru memory history`        | List committed memory updates; use `--limit` to select 1–1000 entries                     |
| `kuru dream`                 | Run explicit consolidation                                                                |
| `kuru undo-dream`            | Restore the previous topology change                                                      |
| `kuru tools`                 | Discover built-in and configured MCP tools                                                |
| `kuru tool NAME --args '{}'` | Invoke a tool with JSON arguments                                                         |
| `kuru serve`                 | Start authenticated loopback [A2A ingress](./a2a)                                         |
| `kuru update`                | [Install an explicit release or source checkout](/guide/installation#update-deliberately) |

Supply `--resume SESSION_ID` with `dream` or `undo-dream` when targeting a saved conversation.

Authentication commands use Kuru's private store, without a Codex CLI or another
application's tokens. `auth` does not create credentials or open project memory;
it is not a live access check. `logout` leaves environment-supplied API keys
unchanged. For API-key access, set `OPENAI_API_KEY` and select
`--provider responses --model MODEL_ID`. See [authentication](/guide/authentication).

## Global options

| Option                   | Effect                                                        |
| ------------------------ | ------------------------------------------------------------- |
| `-C`, `--directory PATH` | Select the project directory                                  |
| `--config PATH`          | Add a final configuration-file layer                          |
| `--data-dir PATH`        | Choose a separate private storage directory                   |
| `--provider NAME`        | Choose `codex`, `responses`, or `demo`                        |
| `--mode NAME`            | Choose a framework                                            |
| `--model ID`             | Choose a model                                                |
| `--effort LEVEL`         | Set reasoning effort                                          |
| `--resume ID`            | Resume a saved conversation                                   |
| `--allow-write`          | Permit built-in file mutation tools                           |
| `--allow-shell`          | Permit shell processes with your account's authority          |
| `--no-dream`             | Disable periodic and session-end dreaming for this invocation |

Global options can be used alongside subcommands. Use `kuru COMMAND --help` for command-specific arguments.
