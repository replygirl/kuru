# Your first conversation

Open a project directory and launch Kuru. Starting with the offline provider makes it easy to explore the interface before signing in:

```sh
cd /path/to/project
kuru --provider demo
```

The demo provider needs no credentials or network access. Kuru includes its verified Dolt engine and licenses; first memory use extracts them locally, and later launches reuse the cache. See [local storage](../concepts/memory#local-storage) for details.

Or select the project without changing directories:

```sh
kuru -C /path/to/project --provider demo
```

## Meet the pool

The welcome portrait shows the selected framework. Type a prompt and press <kbd>Enter</kbd>. Kuru runs a bounded turn through its peers and returns a response under the selected speaking identity.

Try `/parts` to inspect the active peers and relationships. The starting IFS pool includes Self, two managers, two firefighters, and two exiles. Each role offers a different tendency; Self has no supervisory authority over the others.

The wide layout shows peer activity and relationship membership alongside the
conversation. A narrow pane keeps the conversation and editor central. Each
completed reply shows the returned speaker or relationship and its input/output
token counts beneath the reply. Tool-call and peer-round limits are named
separately. An empty model response is labeled as a response outcome, while an
older limited result whose cause was not stored is labeled as an unspecified
legacy limit. Activity reports describe routing and phases without exposing
private peer messages; they do not determine the final reply.

## Make it yours

The controls beside the composer show your current choices:

| Key           | Choice           |
| ------------- | ---------------- |
| <kbd>F2</kbd> | Model            |
| <kbd>F3</kbd> | Reasoning effort |
| <kbd>F4</kbd> | Framework        |

Below them, a standing row keeps the last prepared request's context estimate and this session's cost estimate next to the permission counts, so one screen carries all six. An unknown price reads as `cost unknown`, never as a zero charge or a quota, and the figures are the ones `/cost` and `/permissions` report. Very narrow terminals abbreviate that row rather than drop a control.

Type in a picker to filter, use the arrow keys to move, and press <kbd>Enter</kbd> to select. <kbd>Escape</kbd> closes it. Your unsent draft stays in place. Choices persist for this project, including after quitting and relaunching.

Use <kbd>Alt</kbd>+<kbd>Enter</kbd> for a newline. <kbd>Page Up</kbd> and <kbd>Page Down</kbd> scroll the transcript. During work, <kbd>Escape</kbd> or <kbd>Ctrl</kbd>+<kbd>C</kbd> cancels the operation. While idle, <kbd>Ctrl</kbd>+<kbd>C</kbd> quits.

Cancellation waits for the active operation to settle and keeps your submitted
prompt in the transcript. If an answer completed first, Kuru shows it. Otherwise
Kuru stores a fixed interruption marker that remains visible after later work and
resume. `/retry` reuses only the last submission's exact durable ID, prompt and
target. It can return an already completed result or finish work interrupted
before possible dispatch without duplicating the displayed prompt. It refuses
when an external call may already have been received. Entering another prompt is
new work; Kuru does not retry automatically.

Settings cannot change during active work. Cancel or wait for the turn to finish before choosing another model or framework.

## Work with files

File reading and listing are available inside the project. Writes and shell commands each need explicit permission:

```sh
kuru --allow-write
kuru --allow-write --allow-shell
```

Shell commands use your account's process permissions, including access beyond the project. [Tools and permissions](/reference/tools) explains these boundaries and configured MCP tools.

Kuru reads applicable ancestor `AGENTS.md` files, with nearer instructions taking precedence. Put the relevant instructions in those files; Kuru does not automatically follow arbitrary linked instruction documents.

## Return to the work

`/quit` ends the session. A fresh launch starts a new conversation while preserving project choices and the peers' durable memories. To continue a specific transcript:

```sh
kuru sessions
kuru --resume SESSION_ID
```

See [sessions and dreaming](/concepts/sessions) for memory consolidation, membership changes, and their controls.

## Reduce motion

Framework portraits use fixed ASCII contours with a slow color cycle. The composer stays still while you type. To make ornament static:

```sh
KURU_REDUCED_MOTION=1 kuru
```

Operation timers and meaningful activity continue updating. No special icon font is needed.

For the complete command list, use `/help` or open [commands and shortcuts](/reference/commands).
