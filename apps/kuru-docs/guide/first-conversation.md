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

The wide layout shows peer activity and relationship membership alongside the conversation. A narrow pane keeps the conversation and editor central. Activity reports describe routing and phases without exposing private peer messages.

## Make it yours

The controls beside the composer show your current choices:

| Key           | Choice           |
| ------------- | ---------------- |
| <kbd>F2</kbd> | Model            |
| <kbd>F3</kbd> | Reasoning effort |
| <kbd>F4</kbd> | Framework        |

Type in a picker to filter, use the arrow keys to move, and press <kbd>Enter</kbd> to select. <kbd>Escape</kbd> closes it. Your unsent draft stays in place. Choices persist for this project, including after quitting and relaunching.

Use <kbd>Alt</kbd>+<kbd>Enter</kbd> for a newline. <kbd>Page Up</kbd> and <kbd>Page Down</kbd> scroll the transcript. During work, <kbd>Escape</kbd> or <kbd>Ctrl</kbd>+<kbd>C</kbd> cancels the operation. While idle, <kbd>Ctrl</kbd>+<kbd>C</kbd> quits.

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
