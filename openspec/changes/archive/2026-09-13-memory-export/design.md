## Context

Current memory getters target bounded conversations or named state and read the
selected branch at query time. The existing server can open a validated pool for
an exact Dolt reference, while the platform already provides checked file
publication. A full export needs those boundaries without retaining a SQL
transaction or interpreting storage through runtime policy.

## Goals / Non-Goals

**Goals:** capture one immutable active-main application snapshot; preserve
storage-complete rows and provenance; render one completed snapshot into JSON or
Markdown; and publish only after the complete staged output is validated.

**Non-Goals:** identity filters, a runtime classifier, provider/tool work,
redaction, schema changes, retention, deletion, purge, restore, or a general
store browser.

## Decisions

### Commit-qualified private pool with opaque keyset cursors

`begin_active_export` captures `DOLT_HASHOF('HEAD')`, opens the existing
commit-qualified server pool, checks the resulting hash and historical schema,
and returns a snapshot that retains the pool and provenance. Pages use an opaque
cursor bound to that snapshot, with `None` as the first message position and a
separate state phase. A long-lived transaction or offset pagination was rejected
because it retains server resources or becomes unstable and expensive as data
grows.

### Storage-complete application tables, not classified runtime objects

The export registry names only `messages` and `state` for every supported schema.
Message and state DTOs retain their exact address and payload; state stays a
`serde_json::Value`. Dynamic table discovery and runtime namespace/identity
classification were rejected because they could expose operational authority or
omit unknown application content.

### Application-owned format and checked publication

The app serializes pages into a private completed staging file, validates record
counts and serializer completion, then either copies it to stdout or uses the
checked parent-directory publication boundary with `Publication::New`. Markdown
renders the same JSON-shaped records with length-aware fences or escaped values
so arbitrary stored backticks cannot alter record structure. Direct streaming
from SQL and replacement publication were rejected because a failed later page
could expose a partial successful document or overwrite user data.

## Risks / Trade-offs

- [One stored LONGTEXT row can exceed a page's ordinary byte expectation] →
  bound rows, not bytes, and document that fact.
- [A writer advances `main` during export] → retain only the validated
  commit-qualified pool and verify counts from that same revision.
- [A broken stdout can observe a partial copy] → stage before copying, return
  the write failure, and never label incomplete output successful.

## Operational surface

The command has no provider, secret, network listener, container, worker,
architecture, or connection-limit setting. It uses the existing read-only store
opening and query deadline; `--format json|markdown` selects rendering and an
optional output path selects checked new-file publication. The public result is
one completed snapshot rather than a new service or protocol endpoint.

## Integration contract

Memory returns only typed storage rows, opaque cursors, and immutable
provenance. The CLI owns format version, serialization, staging, stdout, and
file publication; it does not add an SDK route or change provider fixtures.
JSON carries signed sequences and exact string/JSON payloads, while Markdown is
derived from those same typed records without classifying their namespaces.
