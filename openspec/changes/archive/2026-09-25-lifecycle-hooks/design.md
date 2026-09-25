## Context

Kuru already freezes configuration and authority before activation, evaluates every tool operation centrally, owns configured child processes, validates mode-policy speaker selection, and admits P06 tool calls in provider order while independent reads may settle concurrently. It has no lifecycle-hook schema or execution point. The roadmap fixes the allowed semantic results but leaves ordering, process protocol, and failure mechanics to implementation.

## Goals / Non-Goals

**Goals:**

- Add one typed connector-owned command protocol that the runtime can invoke at five fixed lifecycle events.
- Preserve existing trust, permission, mode-policy, tool settlement, context-budget, cancellation, and durable-record authority.
- Make every rewrite, denial, stop, annotation, malformed output, and process failure deterministic and observable without exposing raw command or payload data.

**Non-Goals:**

- A plugin runtime, arbitrary event bus, long-lived hook daemon, hook marketplace, or hook-to-hook API.
- An OS sandbox, new filesystem root, new provider authority, or replacement for permissions and mode policy.
- Rewriting settled tool results, settled answers, durable user input, speaker identity, or historical records.

## Decisions

### Use one versioned single-request command protocol

Each configured hook is an owned one-shot command. Kuru writes one bounded JSON request to stdin, closes it, and accepts exactly one bounded JSON response from stdout; stderr is a bounded diagnostic channel and never protocol. The request carries a format version, event kind, stable invocation/event identifiers, and only the projected data needed for that event. The response is an event-specific allow, deny, rewrite, stop, observe, or annotate result. Unknown fields, multiple values, trailing data, invalid UTF-8, oversized output, and results invalid for that event fail validation.

A long-lived plugin host was rejected because it would introduce shared lifecycle, recovery, authentication, and protocol authority beyond P14. Ad hoc exit-code or text parsing was rejected because it cannot express bounded typed rewrites and annotations safely.

### Run chains in declaration order with event-specific failure rules

Hooks for one event run sequentially in effective configuration order. Each pre hook sees the last valid rewritten value; deny, timeout, spawn failure, malformed output, or invalid rewrite stops that pre chain before external dispatch, and later pre hooks do not run. Post hooks observe the immutable settled value and run in order; one post failure becomes its own bounded hook outcome and does not erase the effect or prevent later post hooks. Annotations retain hook order.

Parallel pre-hook execution was rejected because observable rewrite precedence would depend on completion timing. Stopping the post chain on its first error was rejected because a settled effect cannot be made conditional on an unrelated observer's health.

### Place tool hooks inside P06 admission and settlement

`pre_tool` runs as part of original-order admission before permission evaluation and dispatch. The final rewritten name/arguments are decoded, schema-validated, budgeted, root-checked, and evaluated as a new proposed operation; previous permission or trust decisions do not carry across a changed operation. Calls still eligible for P06 concurrency may execute after their own ordered admission.

`post_tool` runs after the call has one immutable settled result. Per-call post chains may execute as independent settlement work, but annotations and unchanged results rejoin the provider continuation by original call order and call ID. This retains concurrency without allowing completion order to change provider-visible ordering.

Running hooks outside admission was rejected because it could bypass permission checks. Serializing all post hooks globally was rejected because it would unnecessarily remove P06 concurrency.

Dreaming has a separate candidate-local `dream_suggest` proposal path. Its provider calls have actor, session, operation, invocation, and call identities, so `pre_tool` and `post_tool` apply there with no fabricated turn ID. The authored proposal cap is checked before pre hooks; rewritten calls still pass the dream-only name, proposal, self-retirement, and topology validators. Post annotations are written only to the candidate actor namespace after its receipt, and candidate abandonment discards them. Dreams have no admitted user turn or mode-selected speaker, so `pre_turn`, `post_turn`, and `speaker_selected` do not fire there.

### Preserve original records and project annotations separately

`pre_turn` may rewrite only the actor input used for the pending dispatch. It does not replace the durable user/raw record. Every intermediate pre-hook rewrite must satisfy the event shape and next-request bounds before a later hook sees it; the final tool operation still passes its ordinary schema, permission, and root checks. `post_tool` and `post_turn` receive bounded projections of immutable settled values and may produce bounded annotations stored and identified as hook output. The completed answer and journal settle before `post_turn` starts. Its bounded outcome reaches the initiating CLI on stderr or the TUI event subscription separately from that immutable output; an exact completed retry never runs the hook again and does not fabricate a replayed post-turn observation. A post-tool annotation is fitted beside the unchanged result for the receiving actor; a post-turn annotation is eligible only for later context. Both use ordinary visibility and context-fit limits, may be omitted truthfully, and never trigger a provider request.

P11's admission transaction stores the exact one-based raw user-row ordinal in the turn journal. A changed-input retry validates that anchored original when it remains in the bounded raw suffix, then carries an invocation-scoped effective input to actor work. The public transcript projection may omit the pending node; it never guesses from the raw tail or substitutes a later speaker record. Legacy journals lacking a recoverable anchor refuse only a changed-input rewrite on that specific unfinished retry before provider dispatch. Hook annotations use the actor's session-scoped private namespace, including the candidate view for dreams, and uncertain writes reconcile against that same view.

Treating annotations as tool results or assistant answers was rejected because it would falsify settled output. Making every annotation a mandatory context prefix was rejected because accumulated hook output could exceed the model budget.

### Keep speaker authority in mode policy

`speaker_selected` runs only after the mode policy selects and the runtime validates an eligible speaker. Hooks may observe or stop that dispatch. They cannot return an alternate actor, mutate the eligible set or reason, or cause another selection attempt in the same event.

Allowing substitution was rejected because it would create a configured supervisor above equal-peer mode policy.

### Reuse owned command cleanup and prohibit recursive lifecycle dispatch

Connector execution retains the child/tree, bounds time and stdin/stdout/stderr, drains all pipes, and reaps on success, failure, cancellation, or caller loss. Hook commands receive the finite compatibility environment and retained reviewed workspace used by configured launches; Kuru does not describe that process as sandboxed. The owned hook launch adds a child-only origin marker to that finite environment. This is a recursion convention, not an authenticated approval token. A Kuru process started by the hook keeps trust, admission, provider, and final tool authority checks but constructs an inactive lifecycle-hook host, so the nested operation cannot start another hook chain. The marker does not change the parent environment or durable configuration. Hook-output handling itself never invokes hooks. Configuration bounds hook count and aggregate per-turn executions before runtime construction.

Letting a cancelled future drop an unowned child was rejected because it leaks process authority. A generic recursion counter was rejected because hooks have no legitimate nested dispatch path.

## Risks / Trade-offs

- **[Slow hooks add turn latency]** → Enforce per-command and aggregate event/turn deadlines and expose bounded failure outcomes.
- **[A hook sees sensitive event data]** → Send only the event's projected payload through reviewed process authority, keep credentials/private unrelated history out, and document that an external process is not sandboxed.
- **[A rewrite changes permission meaning]** → Discard prior admission decisions and rerun every ordinary validator and evaluator against the exact final value.
- **[Caller cancellation races a settled effect]** → Preserve the immutable effect and its record, terminate/drain any active hook process, and record the hook cancellation separately.
- **[Annotations consume context]** → Apply existing visibility and fit/omission policy, keep them optional, and report omission without recursive provider dispatch.

## Operational surface

Hooks are host-native one-shot child processes launched by the existing Kuru executable on its supported architecture; this change adds no container, daemon, listener, bind address, port, or separately versioned runtime. The configured executable and arguments are part of reviewed workspace authority. Kuru supplies only its finite compatibility environment and structured stdin, so no provider, MCP, native-store, or deployment secret is implicitly forwarded; a deliberately named external secret remains explicit process authority owned by future configuration rather than this protocol.

Each invocation has fixed validated limits for request bytes, stdout, stderr, duration, and child-tree lifetime. Each conversation or dream operation also owns one shared count, active hook wall-time, and annotation-byte budget. Parallel hooks spend overlapping active wall time once; inference and ordinary tools spend none. An owned worker holds its active budget lease through cleanup and reap. The existing native process boundary owns signal, wait, pipe drain, and descendant cleanup on macOS, Linux, and Windows. Hook protocol format versions belong to Kuru and are rejected when unsupported rather than negotiated over a network.
