## Why

Release run 34424298823 generated nonempty bounded notes twice, but both drafts
were discarded solely for exceeding ten bullets. The hard editorial limit blocks
publication and prevents reviewing the actual prose, although the source checks
and all four native builds passed. Cospec's reference release flow uses generated
Communiqué prose directly and does not impose a bullet-count publication gate.

## What Changes

Treat word and bullet targets as writing guidance. Persist complete generated
Markdown for review when it passes the existing bounded regular-file and nonempty
checks. Preserve exact-source context, clean checkout/version/tag checks, timeout,
provider error privacy and atomic output preservation. Update the real Communiqué
fixture tests so formerly discarded readable drafts become reviewable, while
invalid outputs still fail. No extra model retry loop is introduced.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. The durable release specification requires source-grounded generated notes;
the editorial publication gate was an implementation restriction.

## Impact

packages/kuru-delivery/src/notes.rs, its release_notes integration tests,
communique.toml writing guidance and docs/release.md. No dependencies, credentials,
workflow inputs, publication topology or archive format change. The completed
direct-install branch remains separate while this release repair is integrated.

## Surfaces

- [ ] interactive — no interactive control change
- [x] deploy — release notes no longer fail solely on editorial length
- [x] integration — actual pinned Communiqué generation and output contract
- [x] agent-behavior — concision guidance remains advisory
