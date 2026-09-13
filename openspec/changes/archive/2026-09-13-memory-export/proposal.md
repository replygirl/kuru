## Why

Kuru can report its active memory status and recent revisions, but a person
cannot faithfully obtain the complete committed application memory behind one
revision. Existing bounded runtime readers are purpose-specific and cannot
represent unknown namespaces, raw instructions, signed message identifiers, or
unknown JSON fields without silently losing data.

An export must remain a read-only committed snapshot while a writer may advance
`main`. It therefore needs a validated revision-qualified memory reader and a
completed, checked application output before any destination is published.

## What Changes

- Add a paged, revision-pinned active-main export reader for every `messages`
  and `state` row in supported Kuru schemas, with exact provenance and counts.
- Add `kuru memory export` with versioned JSON and an equivalent Markdown
  rendering, private staging, stdout copying, and checked no-replacement file
  publication.
- Document the committed-snapshot boundary, complete storage scope, retained
  history, and output behavior.

## Capabilities

### New Capabilities

<!-- None. -->

### Modified Capabilities

- `versioned-memory`: Add a full active-project committed export that preserves
  application storage records and provenance without mutating memory.
- `public-documentation`: Document the user-facing memory export command and
  its snapshot, output, and retained-history limits.

## Impact

This changes `kuru-memory` public read types and internal commit-qualified page
queries; `apps/kuru-tui` command parsing, private staging, rendering, and
fixtures; and memory documentation. It adds no schema, dependency, provider,
runtime classification, credential, redaction, expiry, deletion, or migration
behavior.

## Surfaces

- [x] interactive — `kuru memory export` produces a user-requested snapshot.
- [ ] deploy — no deployment, workflow, secret, or runtime topology changes.
- [x] integration — JSON is a versioned interchange representation and file
  publication uses the native checked filesystem boundary.
- [ ] agent-behavior — no provider, prompt, tool, or model routing behavior.
