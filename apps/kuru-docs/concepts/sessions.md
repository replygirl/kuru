# Sessions & dreaming

A session is a conversation. Starting Kuru normally creates a new one while retaining the project's parts, durable memories, and saved model, effort, and framework choices.

## Resume a conversation

```sh
kuru sessions
kuru --resume SESSION_ID
```

Session listing does not create a session. Resuming restores the chosen transcript and its framework, even when a different `--mode` was supplied. It does not change the remembered framework for future fresh conversations.

When Kuru chooses a speaker automatically, it selects an eligible peer with the
highest reported activation. Equal activations keep the session's previously
completed speaker when that peer is still tied; otherwise the first stable
identity wins. A caller target and active focus still take precedence. The
completed speaker is saved with the session, and the event trace records the
fixed reason for each selection.

You can resume from a script too:

```sh
kuru --resume SESSION_ID run "Continue from our last turn."
```

## Dreaming

Dreaming consolidates memory and considers changes to membership. Parts propose changes using their own context; the runtime validates the proposals.

- Additions must fit the `max_parts` limit.
- Retirements must preserve at least one active member of each role.
- Retired parts are archived with their memory intact.
- A saved prior topology makes the latest accepted topology change reversible.

Run a dream in the terminal with `/dream`, or target a saved session:

```sh
kuru --resume SESSION_ID dream
kuru --resume SESSION_ID undo-dream
```

The terminal command `/undo-dream` restores the previous topology change. It is not an undo of every action or tool effect from a session.

Every dream writes to an isolated Dolt candidate branch. Its histories, summaries and proposed membership become active together after validation. Cancellation before promotion leaves live memory unchanged. An accepted promotion may finish after cancellation; Kuru reconciles its result before further work. An outdated candidate cannot overwrite later conversations. Undo adds a compensating revision while preserving chats and preferences written afterward. Inspect revisions with `/memory-history` or `kuru memory history`.

Dreaming uses provider calls and can add to the cost of a session. It is a bounded consolidation operation, not an unbounded background process.

A turn's answer is durable before periodic dreaming starts. Its returned event
trace ends at that response; later dream events remain visible as maintenance
activity and cannot suppress the answer. Session-end dreaming has a finite
30-second deadline, after which Kuru still attempts normal actor and tool cleanup.

## Choose the triggers

Three triggers share the same validation path:

| Trigger     | Control                         | Default              |
| ----------- | ------------------------------- | -------------------- |
| Explicit    | `/dream` or the `dream` command | Available on request |
| Periodic    | `dream_every`                   | Every 8 turns        |
| Session end | `dream_on_exit`                 | Enabled              |

Set `dream_every = 0` to disable periodic dreaming and `dream_on_exit = false` to disable the exit trigger. These settings are independent. For one invocation, disable both automatic triggers with:

```sh
kuru --no-dream
```

Explicit dreaming remains available. A scripted conversation prints its completed answer before performing any configured exit dream.

## Script a turn

```sh
kuru --provider demo --mode jungian run "Explore this design."
kuru --provider demo run "Plan the next step." --json
```

JSON output includes the session, speaker, response text, relationship, token counts, budget-limit status, and event trace. `limited` means the round or tool budget constrained the turn; it does not claim the task is complete.

The [configuration reference](/reference/configuration) describes the budgets and where these settings are stored.
