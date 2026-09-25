# Sessions & dreaming

A session is a conversation. Starting Kuru normally creates a new one while retaining the project's parts, durable memories, and saved model, effort, and framework choices.

## Resume a conversation

```sh
kuru sessions
kuru --resume SESSION_ID
kuru --continue
```

Session listing does not create a session. Resuming restores the chosen transcript and its framework, even when a different `--mode` was supplied. It does not change the remembered framework for future fresh conversations.
`--continue` chooses the latest nonremoved session by durable catalog order. A fresh process asks again for session-only tool permissions.

## Manage and export sessions

```sh
kuru sessions rename SESSION_ID "New label"
kuru sessions remove SESSION_ID
kuru sessions restore SESSION_ID
kuru sessions fork SESSION_ID SETTLED_NODE_ID --label "New branch"
kuru sessions export SESSION_ID --format markdown --output transcript.md
kuru sessions export SESSION_ID --format jsonl --output transcript.jsonl
```

Removal hides a session from ordinary listing and resume. Restoration returns
the same ID, transcript and mode; neither action deletes project memory or
revision history. `kuru sessions` reports each session's latest settled
`head_node_id`. The terminal's `/sessions` picker can also select older settled
boundaries: choose a session, press Ctrl+F, then Page Down for older pages.
Forking through a completed or terminally interrupted turn copies the public
prefix into a new session. Pending turns cannot be boundaries. Parent and fork
then grow independently, but both continue using current shared project memory;
forking does not rewind private memory or topology.

Session export contains one public transcript in chronological order, with typed
content, settlement, speaker and fork provenance. Markdown is the default;
JSONL has a manifest followed by one record per line. Without `--output`, it
prints to stdout. A selected existing output can be replaced after an identity
check. Private actor and relationship histories, reasoning summaries, notes and
candidate branches stay outside this export. Use `kuru memory export` for the
full current-memory snapshot. Historical rows without proven speaker or turn
identity show unknown values; Kuru does not guess from current topology.

P11 still allows one conversation driver per project. Concurrent drivers and
live-session presence are covered by the separate concurrent-session work.

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

## Retry the last submitted turn

Each new local CLI or terminal submission receives its own turn ID. Scripted
callers can supply one explicitly:

```sh
kuru --resume SESSION_ID run "Continue from our last turn." --turn-id TURN_ID
```

The terminal command `/retry` uses the session's single durably retained last
submission: its exact ID, prompt and target. If that turn already completed,
Kuru reuses the stored result with no provider or tool call and adds no duplicate
user/answer pair. A retry interrupted before possible dispatch may safely finish
under the same ID. If external work may have been dispatched, Kuru refuses the
retry; entering another prompt creates new work. There is no automatic retry or
arbitrary journal browser.

When an admitted turn settles without an answer, Kuru stores the fixed marker
`Turn interrupted; no completed answer was committed.` It remains visible after
later turns and resume, while staying outside provider conversation context. The
marker does not say whether external work occurred or was rolled back. If the
completed-answer checkpoint wins a cancellation race, the answer appears and no
interruption marker is added.

The turn journal retains every admitted turn without expiry so completed results
remain available for exact retry. Its storage grows in proportion to turns and
intentionally retains some completed-result data also represented in the
assistant transcript. This durable semantic history is separate from the bounded
operational diagnostics ring.

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

JSON output includes the session, speaker, response text, relationship, token
counts, compatibility `limited` status, explicit `limit_reasons`,
`response_outcome`, and the projected event trace. New results distinguish
`tool-calls`, `peer-rounds`, and an `empty` response. An older limited result with
no stored cause reports `legacy-unspecified`.

The [configuration reference](/reference/configuration) describes the budgets and where these settings are stored.
