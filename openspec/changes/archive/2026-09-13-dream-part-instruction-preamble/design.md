## Context

`Framework::builtin` already gives every builtin part the exact equal-peer
preamble followed by `\n\nYour tendency: ` and its static tendency. In contrast,
`Harness::plan_dream` validates a proposed `Add` instruction and copies it
directly into its candidate `Part`. That permits a promoted dream-added part to
miss the framing carried by every builtin, while any repair during loading would
mutate old parts and history outside a new proposal.

## Goals / Non-Goals

**Goals:**

- Construct new builtin and accepted dream-addition instructions through one
  core-owned canonical form without changing existing builtin bytes or IDs.
- Preserve the established raw proposal size validation, individual proposal
  rejection, candidate isolation, promotion, stopped reload, and reversal
  behavior.
- Make the normalization structural and deterministic across UTF-8 boundary
  cases.

**Non-Goals:**

- Interpret, denylist, trim, or otherwise classify a tendency's meaning.
- Repair, reject, or rewrite already persisted parts while loading a topology.
- Change model authority, dream reports, roles, memory schema, candidate
  publication, or provider/runtime policy.

## Decisions

### Canonical structural construction belongs in kuru-core

Core will own the complete canonical prefix and a constructor/validator for a
new part tendency, including the raw incoming 8,192-byte bound. `Framework::builtin`
will use it in a way that retains the current literal instructions and stable ID
seeds exactly. Runtime will retain `Add` name, role, capacity, and topology
validation, then call the same constructor before it places a `Part` in the
candidate topology; it will not duplicate the byte limit, literal, or normalizer.

### Size validation precedes exact-leading normalization

Core checks the incoming `instruction.len() <= 8192` before construction. For a
bounded valid input, it repeatedly removes the complete byte sequence
`PEER_INSTRUCTION + "\n\nYour tendency: "` only while it appears at
the beginning, then rejects a whitespace-only remainder and prepends exactly
one copy. The resulting tail is otherwise copied byte-for-byte. A prefix-like
sequence after authored content, a partial prefix, or whitespace before a
prefix is ordinary authored tail and is not scanned or changed.

This is bounded-domain idempotence: a valid already-canonical input yields the
same canonical instruction. A raw tendency of exactly 8,192 bytes can produce
a larger persisted instruction, and resubmitting that larger result is rejected
by the original incoming bound rather than trimmed to force idempotence.

### Persist only newly accepted additions

Proposal reports continue to retain the original proposal payload. Only the
candidate `Part.instruction` receives the canonical result. The existing
candidate/promotion and compensating-undo paths provide persistence and
reversal; reload reads stored bytes without invoking the new constructor, so
old parts and historical candidate branches are unchanged.

## Risks / Trade-offs

- [A duplicated or near-match prefix is unexpectedly removed] → Match only the
  exact complete ASCII prefix at byte offset zero; test similar prose and
  non-leading copies as unchanged tail.
- [A valid near-limit tendency is mistaken for an invalid wrapped input] → Test
  raw 8,192-byte UTF-8 boundaries separately from a re-submitted oversized
  canonical result, documenting the pre-normalization bound.
- [A rejected add discards a neighboring valid proposal] → Keep the existing
  per-proposal candidate/report loop and test mixed valid and invalid proposals
  through promotion and reload.
